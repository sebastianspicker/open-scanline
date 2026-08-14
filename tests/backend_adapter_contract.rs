use open_scanline::core::{ImageBuffer, PixelFormat, Result, ScanError, ScanMode, ScanRequest};
use open_scanline::device::{DeviceSession, ScanPagesEnd};
use open_scanline::{sane, wia};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier, Mutex, OnceLock,
};
use std::time::{SystemTime, UNIX_EPOCH};

static WIA_OUTPUT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[test]
fn command_builders_keep_values_as_arguments_or_quoted_script_data() {
    let request = ScanRequest {
        width: 600,
        height: 300,
        dpi_x: 300,
        dpi_y: 150,
        ..Default::default()
    };
    let sane = sane::scanimage_command(
        Path::new("/usr/bin/scanimage"),
        "net:scanner with spaces",
        &request,
        Path::new("/tmp/out scan.png"),
    );
    assert_eq!(sane.args[0..2], ["-d", "net:scanner with spaces"]);
    assert!(sane.args.contains(&"--resolution".into()));
    assert!(sane.args.contains(&"300".into()));
    assert!(sane.args.iter().any(|arg| arg == "-x"));
    assert!(sane.args.iter().any(|arg| arg == "50.80"));
    assert!(sane.args.iter().any(|arg| arg == "-y"));
    assert!(sane.args.iter().any(|arg| arg == "50.80"));

    let wia = wia::wia_transfer_command("id'quoted", &request, Path::new("C:/out scan.png"));
    assert_eq!(wia.program, "powershell");
    assert_eq!(wia.args[4], "-Command");
    assert!(wia.args.last().unwrap().contains("$want = 'id''quoted'"));
    assert!(wia.args.last().unwrap().contains("Value = 300"));
    assert!(wia.args.last().unwrap().contains("Value = 600"));
}

#[test]
fn device_parsers_ignore_blank_and_malformed_records() {
    let sane_devices = sane::parse_sane_devices("\nusb:001|Vendor Model\n\n");
    assert_eq!(sane_devices.len(), 1);
    assert_eq!(sane_devices[0].id, "sane:usb:001");
    assert_eq!(sane_devices[0].name, "SANE: Vendor Model");

    let wia_devices = wia::parse_wia_devices("\nABC|Office Scanner\n|ignored\n");
    assert_eq!(wia_devices.len(), 1);
    assert_eq!(wia_devices[0].id, "wia:ABC");
    assert_eq!(wia_devices[0].name, "WIA: Office Scanner");
}

fn fixture_image() -> ImageBuffer {
    ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![10, 20, 30, 40, 50, 60]).unwrap()
}

fn output_path_from_wia(script: &str) -> PathBuf {
    let prefix = "$out = '";
    let start = script.find(prefix).unwrap() + prefix.len();
    let end = script[start..].find('\'').unwrap() + start;
    PathBuf::from(&script[start..end])
}

#[derive(Default)]
struct WiaSuccessRunner {
    calls: Mutex<Vec<wia::CommandSpec>>,
}

impl wia::CommandRunner for WiaSuccessRunner {
    fn run(
        &self,
        spec: &wia::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<wia::CommandOutput> {
        self.calls.lock().unwrap().push(spec.clone());
        std::fs::write(output_path_from_wia(spec.args.last().unwrap()), b"fixture").unwrap();
        Ok(wia::CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

#[derive(Default)]
struct WiaFixtureDecoder {
    decoded: Mutex<Vec<PathBuf>>,
}

impl wia::ImageDecoder for WiaFixtureDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        assert!(
            path.is_file(),
            "runner must materialize the requested output"
        );
        self.decoded.lock().unwrap().push(path.to_path_buf());
        Ok(fixture_image())
    }
}

struct WiaCancelledRunner;

impl wia::CommandRunner for WiaCancelledRunner {
    fn run(
        &self,
        _spec: &wia::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<wia::CommandOutput> {
        Err(ScanError::Cancelled("fixture cancellation".into()))
    }
}

#[derive(Default)]
struct WiaPartialFailureRunner {
    paths: Mutex<Vec<PathBuf>>,
    outcome: WiaPartialFailure,
}

#[derive(Debug, Clone, Copy, Default)]
enum WiaPartialFailure {
    #[default]
    Cancelled,
    Runner,
    Unsuccessful,
}

impl wia::CommandRunner for WiaPartialFailureRunner {
    fn run(
        &self,
        spec: &wia::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<wia::CommandOutput> {
        let path = output_path_from_wia(spec.args.last().unwrap());
        std::fs::write(&path, b"partial output").unwrap();
        self.paths.lock().unwrap().push(path);
        match self.outcome {
            WiaPartialFailure::Cancelled => {
                Err(ScanError::Cancelled("partial cancellation".into()))
            }
            WiaPartialFailure::Runner => Err(ScanError::Unsupported("runner failure".into())),
            WiaPartialFailure::Unsuccessful => Ok(wia::CommandOutput {
                success: false,
                stdout: Vec::new(),
                stderr: b"command failure".to_vec(),
            }),
        }
    }
}

#[derive(Default)]
struct WiaQuotaBreachRunner {
    paths: Mutex<Vec<PathBuf>>,
}

impl wia::CommandRunner for WiaQuotaBreachRunner {
    fn run(
        &self,
        spec: &wia::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<wia::CommandOutput> {
        let path = output_path_from_wia(spec.args.last().unwrap());
        std::fs::write(&path, vec![0_u8; 2 * 1024 * 1024]).unwrap();
        self.paths.lock().unwrap().push(path);
        Ok(wia::CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

struct WiaConcurrentRunner {
    barrier: Barrier,
    paths: Mutex<Vec<PathBuf>>,
    next_image: AtomicUsize,
}

impl WiaConcurrentRunner {
    fn new() -> Self {
        Self {
            barrier: Barrier::new(2),
            paths: Mutex::new(Vec::new()),
            next_image: AtomicUsize::new(0),
        }
    }
}

impl wia::CommandRunner for WiaConcurrentRunner {
    fn run(
        &self,
        spec: &wia::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<wia::CommandOutput> {
        let path = output_path_from_wia(spec.args.last().unwrap());
        let marker = self.next_image.fetch_add(1, Ordering::SeqCst) as u8;
        std::fs::write(&path, [marker, 34, 56]).unwrap();
        self.paths.lock().unwrap().push(path);
        self.barrier.wait();
        Ok(wia::CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

#[derive(Default)]
struct WiaDistinctDecoder {
    paths: Mutex<Vec<PathBuf>>,
}

impl wia::ImageDecoder for WiaDistinctDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        self.paths.lock().unwrap().push(path.to_path_buf());
        ImageBuffer::new(1, 1, PixelFormat::Rgb8, std::fs::read(path)?)
    }
}

#[derive(Default)]
struct FailingDecoder {
    paths: Mutex<Vec<PathBuf>>,
}

impl wia::ImageDecoder for FailingDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        self.paths.lock().unwrap().push(path.to_path_buf());
        Err(ScanError::Unsupported("fixture decode failure".into()))
    }
}

#[test]
fn wia_adapters_drive_successful_decode_and_preserve_cancellation() {
    let _lock = WIA_OUTPUT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    let runner = Arc::new(WiaSuccessRunner::default());
    let decoder = Arc::new(WiaFixtureDecoder::default());
    let session = wia::WiaDeviceSession::new_with_adapters(
        "wia:fixture".into(),
        "fixture".into(),
        false,
        runner.clone(),
        decoder.clone(),
    );
    let image = session
        .scan(&ScanRequest {
            width: 4,
            height: 2,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        })
        .unwrap();
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
    assert_eq!(decoder.decoded.lock().unwrap().len(), 1);

    let cancelled = wia::WiaDeviceSession::new_with_adapters(
        "wia:cancel".into(),
        "cancel".into(),
        false,
        Arc::new(WiaCancelledRunner),
        Arc::new(WiaFixtureDecoder::default()),
    );
    assert!(matches!(
        cancelled.scan(&ScanRequest::default()),
        Err(ScanError::Cancelled(_))
    ));
}

#[test]
fn materialized_output_is_removed_after_decoder_failure() {
    let _lock = WIA_OUTPUT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    let runner = Arc::new(WiaSuccessRunner::default());
    let decoder = Arc::new(FailingDecoder::default());
    let session = wia::WiaDeviceSession::new_with_adapters(
        "wia:decode-failure".into(),
        "decode-failure".into(),
        false,
        runner,
        decoder.clone(),
    );

    assert!(matches!(
        session.scan(&ScanRequest::default()),
        Err(ScanError::Unsupported(message)) if message == "fixture decode failure"
    ));
    let paths = decoder.paths.lock().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(!paths[0].is_file(), "decode failure must clean up output");
}

#[test]
fn injected_runner_over_quota_is_rejected_before_decode_and_cleaned_up() {
    let _lock = WIA_OUTPUT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    let runner = Arc::new(WiaQuotaBreachRunner::default());
    let decoder = Arc::new(WiaFixtureDecoder::default());
    let session = wia::WiaDeviceSession::new_with_adapters(
        "wia:quota".into(),
        "quota".into(),
        false,
        runner.clone(),
        decoder.clone(),
    );

    let result = session.scan(&ScanRequest {
        width: 1,
        height: 1,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    });
    assert!(
        matches!(
            &result,
            Err(ScanError::Unsupported(message)) if message.contains("artifact output exceeded")
        ),
        "unexpected quota result: {result:?}"
    );
    assert!(decoder.decoded.lock().unwrap().is_empty());
    let paths = runner.paths.lock().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(!paths[0].exists(), "quota rejection must clean up output");
}

#[test]
fn wia_acquisitions_use_distinct_paths_and_remove_them_after_concurrent_decodes() {
    let runner = Arc::new(WiaConcurrentRunner::new());
    let decoder = Arc::new(WiaDistinctDecoder::default());
    let first = Arc::new(wia::WiaDeviceSession::new_with_adapters(
        "wia:first".into(),
        "first".into(),
        false,
        runner.clone(),
        decoder.clone(),
    ));
    let second = Arc::new(wia::WiaDeviceSession::new_with_adapters(
        "wia:second".into(),
        "second".into(),
        false,
        runner.clone(),
        decoder.clone(),
    ));
    let request = ScanRequest {
        width: 1,
        height: 1,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let second_request = request.clone();
    let first_task = std::thread::spawn(move || first.scan(&request).unwrap());
    let second_task = std::thread::spawn(move || second.scan(&second_request).unwrap());
    let mut images = [first_task.join().unwrap(), second_task.join().unwrap()];
    images.sort_by_key(|image| image.data[0]);

    assert_eq!(images[0].data, vec![0, 34, 56]);
    assert_eq!(images[1].data, vec![1, 34, 56]);
    let paths = runner.paths.lock().unwrap();
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1]);
    assert!(paths.iter().all(|path| !path.exists()));
    assert!(decoder
        .paths
        .lock()
        .unwrap()
        .iter()
        .all(|path| !path.exists()));
}

#[test]
fn wia_partial_artifacts_are_removed_for_cancellation_and_command_failures() {
    for outcome in [
        WiaPartialFailure::Cancelled,
        WiaPartialFailure::Runner,
        WiaPartialFailure::Unsuccessful,
    ] {
        let runner = Arc::new(WiaPartialFailureRunner {
            paths: Mutex::new(Vec::new()),
            outcome,
        });
        let session = wia::WiaDeviceSession::new_with_adapters(
            "wia:partial".into(),
            "partial".into(),
            false,
            runner.clone(),
            Arc::new(WiaFixtureDecoder::default()),
        );

        assert!(session.scan(&ScanRequest::default()).is_err());
        let paths = runner.paths.lock().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(
            !paths[0].exists(),
            "{outcome:?} must clean up partial output"
        );
    }
}

fn with_fake_scanimage<T>(test: impl FnOnce() -> T) -> T {
    static PATH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = PATH_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_scanimage_fixture_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("scanimage"), b"fixture").unwrap();
    let old_path = std::env::var_os("PATH");
    std::env::set_var("PATH", &directory);
    let result = test();
    match old_path {
        Some(value) => std::env::set_var("PATH", value),
        None => std::env::remove_var("PATH"),
    }
    std::fs::remove_dir_all(directory).unwrap();
    result
}

#[derive(Default)]
struct SaneSuccessRunner {
    calls: Mutex<Vec<sane::CommandSpec>>,
}

impl sane::CommandRunner for SaneSuccessRunner {
    fn run(
        &self,
        spec: &sane::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<sane::CommandOutput> {
        self.calls.lock().unwrap().push(spec.clone());
        let output = spec
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix("--output-file="))
            .unwrap();
        std::fs::write(output, b"fixture").unwrap();
        Ok(sane::CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}

#[derive(Default)]
struct SaneFixtureDecoder {
    decoded: Mutex<Vec<PathBuf>>,
}

impl sane::ImageDecoder for SaneFixtureDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        assert!(
            path.is_file(),
            "runner must materialize the requested output"
        );
        self.decoded.lock().unwrap().push(path.to_path_buf());
        Ok(fixture_image())
    }
}

struct SaneCancelledRunner;

impl sane::CommandRunner for SaneCancelledRunner {
    fn run(
        &self,
        _spec: &sane::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<sane::CommandOutput> {
        Err(ScanError::Cancelled("fixture cancellation".into()))
    }
}

struct SaneBatchRunner {
    calls: Mutex<Vec<sane::CommandSpec>>,
    emitted: u32,
}

impl sane::CommandRunner for SaneBatchRunner {
    fn run(
        &self,
        spec: &sane::CommandSpec,
        _timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<sane::CommandOutput> {
        self.calls.lock().unwrap().push(spec.clone());
        let pattern = spec
            .args
            .iter()
            .find_map(|argument| argument.strip_prefix("--batch="))
            .expect("batch acquisition must provide an output pattern");
        for page in (1..=self.emitted).rev() {
            let path = pattern.replace("%06d", &format!("{page:06}"));
            std::fs::write(path, format!("fixture page {page}")).unwrap();
        }
        Ok(sane::CommandOutput {
            success: false,
            stdout: Vec::new(),
            stderr: b"scanimage: document feeder empty".to_vec(),
        })
    }
}

#[test]
fn sane_adapters_drive_successful_decode_and_preserve_cancellation() {
    with_fake_scanimage(|| {
        let runner = Arc::new(SaneSuccessRunner::default());
        let decoder = Arc::new(SaneFixtureDecoder::default());
        let session = sane::SaneDeviceSession::new_with_adapters(
            "sane:fixture".into(),
            "fixture".into(),
            false,
            runner.clone(),
            decoder.clone(),
        );
        let image = session
            .scan(&ScanRequest {
                width: 4,
                height: 2,
                pixel_format: PixelFormat::Rgb8,
                ..Default::default()
            })
            .unwrap();
        assert_eq!((image.width, image.height), (4, 2));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
        assert_eq!(decoder.decoded.lock().unwrap().len(), 1);
        assert!(decoder
            .decoded
            .lock()
            .unwrap()
            .iter()
            .all(|path| !path.exists()));

        let cancelled = sane::SaneDeviceSession::new_with_adapters(
            "sane:cancel".into(),
            "cancel".into(),
            false,
            Arc::new(SaneCancelledRunner),
            Arc::new(SaneFixtureDecoder::default()),
        );
        assert!(matches!(
            cancelled.scan(&ScanRequest::default()),
            Err(ScanError::Cancelled(_))
        ));
    });
}

#[test]
fn sane_batch_uses_one_process_and_streams_ordered_pages_until_feeder_exhaustion() {
    with_fake_scanimage(|| {
        let runner = Arc::new(SaneBatchRunner {
            calls: Mutex::new(Vec::new()),
            emitted: 2,
        });
        let decoder = Arc::new(SaneFixtureDecoder::default());
        let session = sane::SaneDeviceSession::new_with_adapters(
            "sane:batch-fixture".into(),
            "batch-fixture".into(),
            false,
            runner.clone(),
            decoder.clone(),
        );
        let request = ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            width: 4,
            height: 2,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        };
        let mut images = Vec::new();
        let summary = session
            .scan_pages(&request, 4, &mut |image| {
                images.push(image);
                Ok(())
            })
            .unwrap();

        assert_eq!(summary.emitted, 2);
        assert_eq!(summary.end, ScanPagesEnd::FeederExhausted);
        assert_eq!(images.len(), 2);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0]
            .args
            .iter()
            .any(|argument| argument == "--batch-count=4"));
        assert!(calls[0]
            .args
            .iter()
            .any(|argument| argument == "--duplex=yes"));
        drop(calls);

        let decoded = decoder.decoded.lock().unwrap();
        assert_eq!(decoded.len(), 2);
        assert!(decoded[0].ends_with("page_000001.png"));
        assert!(decoded[1].ends_with("page_000002.png"));
        assert!(decoded.iter().all(|path| !path.exists()));
    });
}

#[test]
fn closed_sessions_take_precedence_over_cancellation_for_both_adapters() {
    let request = ScanRequest::default();
    let sane_session = sane::SaneDeviceSession::new("sane:sim".into(), "sim".into(), true);
    sane_session.close();
    sane_session.cancel();
    assert!(matches!(
        sane_session.scan(&request),
        Err(ScanError::Other(message)) if message == "session closed"
    ));

    let wia_session = wia::WiaDeviceSession::new("wia:sim".into(), "sim".into(), true);
    wia_session.close();
    wia_session.cancel();
    assert!(matches!(
        wia_session.scan(&request),
        Err(ScanError::Other(message)) if message == "session closed"
    ));
}
