use super::*;
use std::hint::black_box;
use std::time::Instant;

const AGGREGATE_PASSES: usize = 4;
const WARMUPS: usize = 3;
const REPETITIONS: usize = 10;
const FIXTURE_SEED: u64 = 1_592_594_996;

struct Fixtures {
    root: PathBuf,
    paths: Vec<PathBuf>,
}

impl Fixtures {
    fn new() -> Self {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "open-scanline-batch-cache-bench-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&root).unwrap();
        let paths = [(1000, 1000, "1mp.jpg"), (6000, 4000, "24mp.jpg")]
            .into_iter()
            .map(|(width, height, name)| write_fixture(&root, width, height, name))
            .collect();
        Self { root, paths }
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write_fixture(root: &Path, width: u32, height: u32, name: &str) -> PathBuf {
    let path = root.join(name);
    let mut data = vec![0_u8; width as usize * height as usize * 3];
    for (index, byte) in data.iter_mut().enumerate() {
        *byte = ((index as u64)
            .wrapping_mul(31)
            .wrapping_add(index as u64 / 97)
            .wrapping_add(FIXTURE_SEED)
            % 251) as u8;
    }
    let image = ImageBuffer::new(width, height, PixelFormat::Rgb8, data).unwrap();
    crate::infrastructure::media::save_image(&path, &image, Some(150), Some(82)).unwrap();
    path
}

fn run_once(paths: &[PathBuf], cache_limit: Option<usize>) -> (u128, usize) {
    let started = Instant::now();
    let decode_count = match cache_limit {
        Some(limit) => {
            let mut loader = SharedPageLoader::with_limit(limit);
            for _ in 0..AGGREGATE_PASSES {
                consume_with_cache(paths, &mut loader);
            }
            loader.decode_count()
        }
        None => {
            let mut decode_count = 0;
            for _ in 0..AGGREGATE_PASSES {
                for path in paths {
                    let image = load_image(path).unwrap();
                    black_box((image.width, image.height, image.data.first().copied()));
                    decode_count += 1;
                }
            }
            decode_count
        }
    };
    (started.elapsed().as_nanos(), decode_count)
}

fn consume_with_cache(paths: &[PathBuf], loader: &mut SharedPageLoader) {
    for path in paths {
        let image = loader.load(path, None).unwrap();
        black_box((image.width, image.height, image.data.first().copied()));
    }
}

fn collect_samples(paths: &[PathBuf], cache_limit: Option<usize>) -> (Vec<u128>, Vec<usize>) {
    for _ in 0..WARMUPS {
        black_box(run_once(paths, cache_limit));
    }
    (0..REPETITIONS)
        .map(|_| run_once(paths, cache_limit))
        .unzip()
}

#[test]
#[ignore = "release-only JSON benchmark: large-image decoding is intentionally expensive"]
fn release_batch_aggregate_cache_benchmark_json() {
    assert!(
        !cfg!(debug_assertions),
        "run this ignored benchmark with --release"
    );
    let fixtures = Fixtures::new();
    let (uncached_latency_ns, uncached_decode_counts) = collect_samples(&fixtures.paths, None);
    let (cached_latency_ns, cached_decode_counts) =
        collect_samples(&fixtures.paths, Some(MAX_CACHE_BYTES));
    println!(
        "{}",
        serde_json::json!({
            "benchmark": "batch-aggregate-decoding",
            "fixture_pixels": [1_000_000, 24_000_000],
            "fixture_generator": {
                "version": 1,
                "seed": FIXTURE_SEED,
                "formula": "((byte_index * 31) + (byte_index / 97) + seed) mod 251",
                "jpeg_quality": 82,
                "dpi": 150,
            },
            "aggregate_passes": AGGREGATE_PASSES,
            "warmups": WARMUPS,
            "repetitions": REPETITIONS,
            "uncached": {
                "latency_ns": uncached_latency_ns,
                "decode_counts": uncached_decode_counts,
            },
            "cached": {
                "latency_ns": cached_latency_ns,
                "decode_counts": cached_decode_counts,
            },
            "ocr_model_reads": "covered by the OCR job factory benchmark"
        })
    );
}
