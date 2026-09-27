use super::*;
use crate::infrastructure::media::ocr::model_pack::{self, ModelPairValidator};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct AcceptModels;

impl ModelPairValidator for AcceptModels {
    fn validate(&self, _: &Path, _: &Path) -> Result<()> {
        Ok(())
    }
}

struct FixtureRunner;

impl OcrsRunner for FixtureRunner {
    fn recognize(&self, _: &ImageBuffer, detection: &[u8], recognition: &[u8]) -> Result<String> {
        assert_eq!(detection, b"detector fixture");
        assert_eq!(recognition, b"recognizer fixture");
        Ok("PRINTED LATIN".into())
    }
}

struct FakeFactory {
    constructions: Arc<AtomicUsize>,
    inferences: Arc<AtomicUsize>,
    cancel_on_construct: Option<CancellationToken>,
    cancel_on_infer: Option<CancellationToken>,
}

impl EngineFactory for FakeFactory {
    fn construct(
        &self,
        detection: Vec<u8>,
        recognition: Vec<u8>,
    ) -> Result<Box<dyn EngineRuntime>> {
        self.constructions.fetch_add(1, Ordering::SeqCst);
        if detection.starts_with(b"invalid") || recognition.starts_with(b"invalid") {
            return Err(ScanError::Invalid("fixture construction rejected".into()));
        }
        if let Some(token) = &self.cancel_on_construct {
            token.cancel();
        }
        Ok(Box::new(FakeEngine {
            inferences: Arc::clone(&self.inferences),
            cancel_on_infer: self.cancel_on_infer.clone(),
        }))
    }
}

struct FakeEngine {
    inferences: Arc<AtomicUsize>,
    cancel_on_infer: Option<CancellationToken>,
}

impl EngineRuntime for FakeEngine {
    fn recognize(&self, _: &ImageBuffer) -> Result<String> {
        self.inferences.fetch_add(1, Ordering::SeqCst);
        if let Some(token) = &self.cancel_on_infer {
            token.cancel();
        }
        Ok("fixture text".into())
    }
}

struct SequenceLoader {
    pairs: Mutex<Vec<Result<Option<model_pack::VerifiedModelPair>>>>,
    reads: Arc<AtomicUsize>,
}

impl ModelLoader for SequenceLoader {
    fn load(&self, _: Option<&CancellationToken>) -> Result<Option<model_pack::VerifiedModelPair>> {
        self.reads.fetch_add(2, Ordering::SeqCst);
        self.pairs.lock().unwrap().remove(0)
    }
}

struct RootLoader(PathBuf);

impl ModelLoader for RootLoader {
    fn load(
        &self,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<model_pack::VerifiedModelPair>> {
        model_pack::active_verified_model_pair_in(&self.0, cancellation)
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-ocr-job-{label}-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn install(&self, suffix: &str) -> model_pack::InstalledModelPack {
        self.install_bytes(
            format!("detection-{suffix}").as_bytes(),
            format!("recognition-{suffix}").as_bytes(),
        )
    }

    fn install_bytes(
        &self,
        detection_bytes: &[u8],
        recognition_bytes: &[u8],
    ) -> model_pack::InstalledModelPack {
        let id = format!("{}-{}", hex(detection_bytes), hex(recognition_bytes));
        let detection = self.0.join(format!("detection-{id}"));
        let recognition = self.0.join(format!("recognition-{id}"));
        std::fs::write(&detection, detection_bytes).unwrap();
        std::fs::write(&recognition, recognition_bytes).unwrap();
        model_pack::install_in(
            &self.0.join("installed"),
            &detection,
            &recognition,
            &AcceptModels,
        )
        .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hex(bytes: &[u8]) -> String {
    format!("{:x}", sha2::Sha256::digest(bytes))
}

fn job(loader: Arc<dyn ModelLoader>, factory: FakeFactory) -> OcrsJob {
    OcrsJob {
        state: EnabledJob {
            loader,
            factory: Arc::new(factory),
            cache: Mutex::new(None),
        },
    }
}

fn factory() -> (FakeFactory, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let constructions = Arc::new(AtomicUsize::new(0));
    let inferences = Arc::new(AtomicUsize::new(0));
    (
        benchmark_factory(&constructions, &inferences),
        constructions,
        inferences,
    )
}

fn benchmark_factory(
    constructions: &Arc<AtomicUsize>,
    inferences: &Arc<AtomicUsize>,
) -> FakeFactory {
    FakeFactory {
        constructions: Arc::clone(constructions),
        inferences: Arc::clone(inferences),
        cancel_on_construct: None,
        cancel_on_infer: None,
    }
}

fn pair(key: &str) -> Result<Option<model_pack::VerifiedModelPair>> {
    Ok(Some(model_pack::VerifiedModelPair {
        key: key.into(),
        detection: format!("detection-{key}").into_bytes(),
        recognition: format!("recognition-{key}").into_bytes(),
    }))
}

#[test]
fn injected_runner_receives_exact_verified_pack_bytes() {
    let scratch = Scratch::new("runner");
    let pack = scratch.install_bytes(b"detector fixture", b"recognizer fixture");
    let result = recognize_pack_with_runner(&sample(), None, &pack, &FixtureRunner).unwrap();
    assert_eq!(result.text, "PRINTED LATIN");
}

#[test]
fn job_reads_both_models_per_page_and_constructs_once_per_content_key() {
    let reads = Arc::new(AtomicUsize::new(0));
    let loader = Arc::new(SequenceLoader {
        pairs: Mutex::new(vec![pair("one"), pair("one"), pair("two")]),
        reads: Arc::clone(&reads),
    });
    let (factory, constructions, inferences) = factory();
    let job = job(loader, factory);
    for _ in 0..3 {
        job.recognize(&sample(), "eng", None).unwrap();
    }
    assert_eq!(reads.load(Ordering::SeqCst), 6);
    assert_eq!(constructions.load(Ordering::SeqCst), 2);
    assert_eq!(inferences.load(Ordering::SeqCst), 3);
}

#[test]
fn real_pack_selection_reads_pointer_manifest_and_each_model_once_per_page() {
    let scratch = Scratch::new("actual-reads");
    let pack = scratch.install("one");
    let (factory, constructions, _) = factory();
    let root = scratch.0.join("installed");
    let job = job(Arc::new(RootLoader(root.clone())), factory);
    model_pack::reset_test_read_counts();
    job.recognize(&sample(), "eng", None).unwrap();
    job.recognize(&sample(), "eng", None).unwrap();
    assert_eq!(model_pack::test_read_count(&root.join("active.json")), 2);
    assert_eq!(
        model_pack::test_read_count(&pack.root.join("manifest.json")),
        2
    );
    assert_eq!(
        model_pack::test_read_count(&pack.root.join("detection.rten")),
        2
    );
    assert_eq!(
        model_pack::test_read_count(&pack.root.join("recognition.rten")),
        2
    );
    assert_eq!(constructions.load(Ordering::SeqCst), 1);
}

#[test]
fn corrupt_replacement_errors_then_restored_pointer_reuses_cached_engine() {
    let scratch = Scratch::new("replacement");
    let first = scratch.install("one");
    let (factory, constructions, inferences) = factory();
    let root = scratch.0.join("installed");
    let job = job(Arc::new(RootLoader(root.clone())), factory);
    job.recognize(&sample(), "eng", None).unwrap();
    let second = scratch.install("two");
    std::fs::write(second.root.join("detection.rten"), b"corrupt").unwrap();
    assert!(job.recognize(&sample(), "eng", None).is_err());
    assert_eq!(inferences.load(Ordering::SeqCst), 1);
    std::fs::write(
        root.join("active.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format_version": 1,
            "pack": first.id
        }))
        .unwrap(),
    )
    .unwrap();
    job.recognize(&sample(), "eng", None).unwrap();
    assert_eq!(constructions.load(Ordering::SeqCst), 1);
}

#[test]
fn invalid_replacement_engine_never_runs_old_cached_engine() {
    let reads = Arc::new(AtomicUsize::new(0));
    let loader = Arc::new(SequenceLoader {
        pairs: Mutex::new(vec![
            pair("one"),
            Ok(Some(model_pack::VerifiedModelPair {
                key: "invalid".into(),
                detection: b"invalid replacement".to_vec(),
                recognition: b"recognition".to_vec(),
            })),
            pair("one"),
        ]),
        reads,
    });
    let (factory, constructions, inferences) = factory();
    let job = job(loader, factory);
    job.recognize(&sample(), "eng", None).unwrap();
    assert!(job.recognize(&sample(), "eng", None).is_err());
    assert_eq!(inferences.load(Ordering::SeqCst), 1);
    job.recognize(&sample(), "eng", None).unwrap();
    assert_eq!(constructions.load(Ordering::SeqCst), 3);
    assert_eq!(inferences.load(Ordering::SeqCst), 2);
}

#[test]
fn cancellation_is_checked_after_construction_and_inference() {
    let construction_token = CancellationToken::new();
    let construction_job = job(
        Arc::new(SequenceLoader {
            pairs: Mutex::new(vec![pair("one")]),
            reads: Arc::new(AtomicUsize::new(0)),
        }),
        FakeFactory {
            constructions: Arc::new(AtomicUsize::new(0)),
            inferences: Arc::new(AtomicUsize::new(0)),
            cancel_on_construct: Some(construction_token.clone()),
            cancel_on_infer: None,
        },
    );
    assert!(matches!(
        construction_job.recognize(&sample(), "eng", Some(&construction_token)),
        Err(ScanError::Cancelled(_))
    ));

    let inference_token = CancellationToken::new();
    let inference_job = job(
        Arc::new(SequenceLoader {
            pairs: Mutex::new(vec![pair("one")]),
            reads: Arc::new(AtomicUsize::new(0)),
        }),
        FakeFactory {
            constructions: Arc::new(AtomicUsize::new(0)),
            inferences: Arc::new(AtomicUsize::new(0)),
            cancel_on_construct: None,
            cancel_on_infer: Some(inference_token.clone()),
        },
    );
    assert!(matches!(
        inference_job.recognize(&sample(), "eng", Some(&inference_token)),
        Err(ScanError::Cancelled(_))
    ));
}

#[test]
fn pre_cancelled_job_does_not_read_models_or_construct_engine() {
    let reads = Arc::new(AtomicUsize::new(0));
    let loader = Arc::new(SequenceLoader {
        pairs: Mutex::new(Vec::new()),
        reads: Arc::clone(&reads),
    });
    let (factory, constructions, _) = factory();
    let job = job(loader, factory);
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        job.recognize(&sample(), "eng", Some(&token)),
        Err(ScanError::Cancelled(_))
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(constructions.load(Ordering::SeqCst), 0);
}

fn benchmark_job(page_count: usize, shared_job: bool) -> (u128, usize, usize, usize, usize) {
    let scratch = Scratch::new("benchmark");
    let pack = scratch.install("stable");
    let root = scratch.0.join("installed");
    let constructions = Arc::new(AtomicUsize::new(0));
    let inferences = Arc::new(AtomicUsize::new(0));
    model_pack::reset_test_read_counts();
    let started = std::time::Instant::now();
    if shared_job {
        let export_job = job(
            Arc::new(RootLoader(root.clone())),
            benchmark_factory(&constructions, &inferences),
        );
        for _ in 0..page_count {
            std::hint::black_box(export_job.recognize(&sample(), "eng", None).unwrap());
        }
    } else {
        for _ in 0..page_count {
            let page_job = job(
                Arc::new(RootLoader(root.clone())),
                benchmark_factory(&constructions, &inferences),
            );
            std::hint::black_box(page_job.recognize(&sample(), "eng", None).unwrap());
        }
    }
    (
        started.elapsed().as_nanos(),
        model_pack::test_read_count(&root.join("active.json")),
        model_pack::test_read_count(&pack.root.join("manifest.json")),
        model_pack::test_read_count(&pack.root.join("detection.rten"))
            + model_pack::test_read_count(&pack.root.join("recognition.rten")),
        constructions.load(Ordering::SeqCst),
    )
}

#[test]
#[ignore = "release-only JSON evidence for OCR model reads and engine reuse"]
fn release_ocr_job_cache_benchmark_json() {
    assert!(!cfg!(debug_assertions), "run this benchmark with --release");
    for _ in 0..3 {
        std::hint::black_box((benchmark_job(100, false), benchmark_job(100, true)));
    }
    let fresh = (0..10)
        .map(|_| benchmark_job(100, false))
        .collect::<Vec<_>>();
    let shared = (0..10)
        .map(|_| benchmark_job(100, true))
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::json!({
            "benchmark": "ocr-export-job-cache",
            "pages_per_repetition": 100,
            "warmups": 3,
            "repetitions": 10,
            "fixture": "installed temporary pack with fake model bytes and injected engine",
            "fresh_job_per_page": {
                "latency_ns": fresh.iter().map(|sample| sample.0).collect::<Vec<_>>(),
                "active_pointer_reads": fresh.iter().map(|sample| sample.1).collect::<Vec<_>>(),
                "manifest_reads": fresh.iter().map(|sample| sample.2).collect::<Vec<_>>(),
                "model_file_reads": fresh.iter().map(|sample| sample.3).collect::<Vec<_>>(),
                "injected_engine_constructions": fresh.iter().map(|sample| sample.4).collect::<Vec<_>>()
            },
            "shared_export_job": {
                "latency_ns": shared.iter().map(|sample| sample.0).collect::<Vec<_>>(),
                "active_pointer_reads": shared.iter().map(|sample| sample.1).collect::<Vec<_>>(),
                "manifest_reads": shared.iter().map(|sample| sample.2).collect::<Vec<_>>(),
                "model_file_reads": shared.iter().map(|sample| sample.3).collect::<Vec<_>>(),
                "injected_engine_constructions": shared.iter().map(|sample| sample.4).collect::<Vec<_>>()
            }
        })
    );
}

#[test]
fn optional_real_model_pair_recognizes_printed_latin_fixture() {
    let Ok(root) = std::env::var("OPEN_SCANLINE_OCR_REAL_MODEL_TEST_DIR") else {
        return;
    };
    let root = PathBuf::from(root);
    let detection = root.join("detection.rten");
    let recognition = root.join("recognition.rten");
    validate_model_files(&detection, &recognition).unwrap();
    let image = crate::infrastructure::media::load_image(root.join("fixture.png")).unwrap();
    let expected = std::fs::read_to_string(root.join("expected.txt")).unwrap();
    let detection = model_pack::read_model_source_bytes(&detection, "detection").unwrap();
    let recognition = model_pack::read_model_source_bytes(&recognition, "recognition").unwrap();
    let actual = LocalOcrsRunner
        .recognize(&image, &detection, &recognition)
        .unwrap();
    assert!(actual.contains(expected.trim()));
}
