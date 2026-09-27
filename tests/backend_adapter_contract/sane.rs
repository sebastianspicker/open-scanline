use super::support::*;
use open_scanline::device::ScanPagesResult;

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

#[derive(Clone, Copy)]
enum SaneMaintenanceOutcome {
    Success,
    Nonzero,
    Cancelled,
    TimedOut,
}

struct SaneMaintenanceRunner {
    calls: Mutex<Vec<sane::CommandSpec>>,
    timeouts: Mutex<Vec<std::time::Duration>>,
    outcome: SaneMaintenanceOutcome,
}

impl SaneMaintenanceRunner {
    fn new(outcome: SaneMaintenanceOutcome) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            outcome,
        }
    }
}

impl sane::CommandRunner for SaneMaintenanceRunner {
    fn run(
        &self,
        spec: &sane::CommandSpec,
        timeout: std::time::Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<sane::CommandOutput> {
        self.calls.lock().unwrap().push(spec.clone());
        self.timeouts.lock().unwrap().push(timeout);
        if spec.args.iter().any(|argument| argument == "-A") {
            return Ok(sane::CommandOutput {
                success: true,
                stdout: b"--calibrate\n--autofocus\n--focusx [0..200]\n--focusy [0..100]\n"
                    .to_vec(),
                stderr: Vec::new(),
            });
        }
        match self.outcome {
            SaneMaintenanceOutcome::Success => Ok(sane::CommandOutput {
                success: true,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }),
            SaneMaintenanceOutcome::Nonzero => Ok(sane::CommandOutput {
                success: false,
                stdout: Vec::new(),
                stderr: b"fixture nonzero".to_vec(),
            }),
            SaneMaintenanceOutcome::Cancelled => {
                Err(ScanError::Cancelled("fixture cancellation".into()))
            }
            SaneMaintenanceOutcome::TimedOut => Err(ScanError::Other("fixture timeout".into())),
        }
    }
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
fn sane_maintenance_uses_inspection_then_bounded_no_scan_commands() {
    with_fake_scanimage(|| {
        let successful = Arc::new(SaneMaintenanceRunner::new(SaneMaintenanceOutcome::Success));
        let session = sane::SaneDeviceSession::new_with_adapters(
            "sane:maintenance".into(),
            "maintenance".into(),
            false,
            successful.clone(),
            Arc::new(SaneFixtureDecoder::default()),
        );
        assert!(session.maintenance_capabilities().focus.supports_point());
        assert_eq!(session.focus(0.25, 0.5)["status"], "focused");
        assert_eq!(session.calibrate()["status"], "calibrated");
        let calls = successful.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].args, ["-d", "maintenance", "-A"]);
        assert_eq!(
            calls[1].args,
            [
                "-d",
                "maintenance",
                "--focusx",
                "50",
                "--focusy",
                "50",
                "--autofocus",
                "--dont-scan",
            ]
        );
        assert_eq!(
            calls[2].args,
            ["-d", "maintenance", "--calibrate", "--dont-scan"]
        );
        assert_eq!(successful.timeouts.lock().unwrap()[1].as_secs(), 20);
        assert_eq!(successful.timeouts.lock().unwrap()[2].as_secs(), 20);

        assert_maintenance_failures();
    });
}

fn assert_maintenance_failures() {
    for outcome in [
        SaneMaintenanceOutcome::Nonzero,
        SaneMaintenanceOutcome::Cancelled,
        SaneMaintenanceOutcome::TimedOut,
    ] {
        let session = sane::SaneDeviceSession::new_with_adapters(
            "sane:maintenance".into(),
            "maintenance".into(),
            false,
            Arc::new(SaneMaintenanceRunner::new(outcome)),
            Arc::new(SaneFixtureDecoder::default()),
        );
        let result = session.calibrate();
        assert!(!result["ok"].as_bool().unwrap());
        assert!(matches!(
            result["status"].as_str(),
            Some("error") | Some("cancelled")
        ));
    }
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

        assert_batch_summary(summary, &images);
        assert_batch_command(&runner);
        assert_decoded_batch(&decoder);
    });
}

fn assert_batch_summary(summary: ScanPagesResult, images: &[ImageBuffer]) {
    assert_eq!(summary.emitted, 2);
    assert_eq!(summary.end, ScanPagesEnd::FeederExhausted);
    assert_eq!(images.len(), 2);
}

fn assert_batch_command(runner: &SaneBatchRunner) {
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
}

fn assert_decoded_batch(decoder: &SaneFixtureDecoder) {
    let decoded = decoder.decoded.lock().unwrap();
    assert_eq!(decoded.len(), 2);
    assert!(decoded[0].ends_with("page_000001.png"));
    assert!(decoded[1].ends_with("page_000002.png"));
    assert!(decoded.iter().all(|path| !path.exists()));
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
