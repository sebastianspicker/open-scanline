use super::support::*;

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
