//! External consumer characterization for the intentionally preserved facades.
//!
//! Keep this focused on public names, constructors, and signatures.  Runtime
//! behavior is covered elsewhere; this file should remain valid across an
//! change to the internal module layout.

use open_scanline::batch::{self, BatchScanArgs};
use open_scanline::core::{ImageBuffer, PixelFormat, Result, ScanMode, ScanRequest};
use open_scanline::device::{
    AnySession, BackendInfo, CancellationToken, DeviceInfo, DeviceOpenPolicy, DeviceSession,
    FileDeviceSession, MockDevice, MockDeviceSession, ScanPagesEnd, ScanPagesResult,
};
use open_scanline::export::{ExportOptions, OcrEngine};
use open_scanline::imaging::{self, PdfOptions};
use open_scanline::ml::{
    OnnxInferenceOptions, OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
};
use open_scanline::packaging::PackagingOptions;
use open_scanline::process::{self, ProcessOptions};
use open_scanline::scan::{self, ScanToFileArgs};
use open_scanline::{sane, wia};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn assert_device_session<T: DeviceSession>() {}

#[derive(Default)]
struct InertRunner;

impl sane::CommandRunner for InertRunner {
    fn run(
        &self,
        _spec: &sane::CommandSpec,
        _timeout: Duration,
        _cancelled: &Mutex<bool>,
    ) -> Result<sane::CommandOutput> {
        Err(open_scanline::core::ScanError::Other(
            "inert external-contract runner".into(),
        ))
    }
}

#[derive(Default)]
struct InertDecoder;

impl sane::ImageDecoder for InertDecoder {
    fn decode(&self, _path: &Path) -> Result<ImageBuffer> {
        Err(open_scanline::core::ScanError::Other(
            "inert external-contract decoder".into(),
        ))
    }
}

#[test]
fn preserved_public_facades_remain_usable_by_external_callers() {
    assert_device_session::<AnySession>();
    assert_device_session::<MockDeviceSession>();
    assert_device_session::<FileDeviceSession>();
    assert_device_session::<sane::SaneDeviceSession>();
    assert_device_session::<wia::WiaDeviceSession>();

    let cancellation = CancellationToken::new();
    assert!(!cancellation.is_cancelled());
    let shared_flag = cancellation.as_arc();
    cancellation.cancel();
    assert!(cancellation.is_cancelled());
    assert!(shared_flag.load(std::sync::atomic::Ordering::SeqCst));

    let device = DeviceInfo::new("compat:device", "Compatibility Scanner", "test");
    assert_eq!(device.id, "compat:device");
    let backend = BackendInfo {
        id: "compat".into(),
        name: "Compatibility backend".into(),
        available: true,
    };
    assert!(backend.available);
    assert_eq!(MockDevice::DEVICE_ID, "mock");
    assert_eq!(
        ScanPagesResult::limit_reached(2).end,
        ScanPagesEnd::LimitReached
    );
    assert_eq!(
        ScanPagesResult::feeder_exhausted(1).end,
        ScanPagesEnd::FeederExhausted
    );

    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![1, 2, 3])
        .expect("public image constructor accepts packed RGB data");
    let request = ScanRequest {
        mode: ScanMode::Reflective,
        ..ScanRequest::default()
    };
    assert_eq!(request.dpi_x, 150);

    let export = ExportOptions {
        pdf_password: None,
        searchable_pdf: true,
        ocr_language: "eng".into(),
        ocr_engine: OcrEngine::Offline,
        scanner_profile: None,
    };
    assert!(export.searchable_pdf);
    let pdf = PdfOptions {
        dpi: 150,
        title: "compatibility".into(),
        password: None,
        searchable_pages: None,
    };
    assert_eq!(pdf.dpi, 150);

    let onnx_options = OnnxInferenceOptions {
        input_name: Some("pixels".into()),
        layout: OnnxInputLayout::Nchw,
        normalization: OnnxNormalization::ZeroToOne,
    };
    let report = OnnxReport {
        ok: true,
        engine: "compat".into(),
        model: PathBuf::from("model.onnx"),
        input_name: "pixels".into(),
        input_shape: vec![1, 3, 1, 1],
        layout: onnx_options.layout,
        normalization: onnx_options.normalization,
        outputs: vec![OnnxOutputSummary {
            shape: vec![1, 3, 1, 1],
            datum_type: "f32".into(),
            size: 3,
            min: Some(0.0),
            max: Some(1.0),
            mean: Some(0.5),
            sample: vec![0.0, 0.5, 1.0],
        }],
    };
    assert_eq!(report.as_dict()["input_name"], "pixels");

    let package = PackagingOptions {
        binary: PathBuf::from("open-scanline"),
        out: PathBuf::from("open-scanline.zip"),
    };
    assert_eq!(
        package.out.extension().and_then(|value| value.to_str()),
        Some("zip")
    );

    let scan_args = ScanToFileArgs {
        out: PathBuf::from("scan.png"),
        ..ScanToFileArgs::default()
    };
    let batch_args = BatchScanArgs {
        out_dir: PathBuf::from("pages"),
        ..BatchScanArgs::default()
    };
    let process_options = ProcessOptions {
        src: PathBuf::from("source.png"),
        dst: PathBuf::from("output.png"),
        pipeline: Default::default(),
        quality: Some(90),
    };
    assert_eq!(scan_args.out, PathBuf::from("scan.png"));
    assert_eq!(batch_args.out_dir, PathBuf::from("pages"));
    assert_eq!(process_options.quality, Some(90));

    let _: fn(ScanToFileArgs) -> Result<PathBuf> = scan::run_scan_to_file;
    let _: fn(ScanToFileArgs, CancellationToken) -> Result<PathBuf> =
        scan::run_scan_to_file_with_token;
    let _: fn(ScanToFileArgs, &ExportOptions) -> Result<PathBuf> =
        scan::run_scan_to_file_with_export_options;
    let _: fn(ScanToFileArgs, &ExportOptions, CancellationToken) -> Result<PathBuf> =
        scan::run_scan_to_file_with_export_options_and_token;
    let _: fn(
        ScanToFileArgs,
        &ExportOptions,
        CancellationToken,
        DeviceOpenPolicy,
    ) -> Result<PathBuf> = scan::run_scan_to_file_with_export_options_and_token_and_policy;
    let _: fn(BatchScanArgs) -> Result<Vec<PathBuf>> = batch::run_batch_scan;
    let _: fn(BatchScanArgs, CancellationToken) -> Result<Vec<PathBuf>> =
        batch::run_batch_scan_with_token;
    let _: fn(BatchScanArgs, &ExportOptions) -> Result<Vec<PathBuf>> =
        batch::run_batch_scan_with_export_options;
    let _: fn(BatchScanArgs, &ExportOptions, CancellationToken) -> Result<Vec<PathBuf>> =
        batch::run_batch_scan_with_export_options_and_token;
    let _: fn(
        BatchScanArgs,
        &ExportOptions,
        CancellationToken,
        DeviceOpenPolicy,
    ) -> Result<Vec<PathBuf>> = batch::run_batch_scan_with_export_options_and_token_and_policy;
    let _: fn(
        BatchScanArgs,
        &ExportOptions,
        Option<&batch::BatchCancelCheck>,
    ) -> Result<Vec<PathBuf>> = batch::run_batch_scan_with_export_options_and_cancel;
    let _: fn(&ProcessOptions) -> Result<PathBuf> = process::process_image_file;
    let _: fn(&ProcessOptions, CancellationToken) -> Result<PathBuf> =
        process::process_image_file_with_token;
    let _: fn(&ProcessOptions, &ExportOptions) -> Result<PathBuf> =
        process::process_image_file_with_export_options;
    let _: fn(&ProcessOptions, &ExportOptions, CancellationToken) -> Result<PathBuf> =
        process::process_image_file_with_export_options_and_token;

    let runner: Arc<dyn sane::CommandRunner> = Arc::new(InertRunner);
    let decoder: Arc<dyn sane::ImageDecoder> = Arc::new(InertDecoder);
    let sane_session = sane::SaneDeviceSession::new_with_adapters(
        "sane:compat".into(),
        "compat".into(),
        false,
        Arc::clone(&runner),
        Arc::clone(&decoder),
    );
    let wia_session = wia::WiaDeviceSession::new_with_adapters(
        "wia:compat".into(),
        "compat".into(),
        false,
        runner,
        decoder,
    );
    assert_eq!(sane_session.device_id, "sane:compat");
    assert_eq!(wia_session.device_id, "wia:compat");

    assert!(sane::parse_sane_devices("").is_empty());
    assert!(wia::parse_wia_devices("").is_empty());
    assert_eq!(
        sane::scanimage_command(
            Path::new("scanimage"),
            "compat",
            &request,
            Path::new("scan.png")
        )
        .program,
        "scanimage"
    );
    assert!(
        wia::wia_transfer_command("compat", &request, Path::new("scan.png"))
            .args
            .iter()
            .any(|argument| argument == "-Command")
    );

    // These calls prove the public, generic PDF API without producing files.
    if false {
        let pages = vec![PathBuf::from("page.png")];
        let _ = imaging::save_pdf_with_options(Path::new("out.pdf"), &[image], &pdf);
        let _ = imaging::save_pdf_with_options_and_cancellation(
            Path::new("out.pdf"),
            &[],
            &pdf,
            Some(&CancellationToken::new()),
        );
        let _ = imaging::save_multipage_pdf(&pages, Path::new("out.pdf"), &pdf);
        let _ =
            imaging::save_multipage_pdf_with_cancellation(&pages, Path::new("out.pdf"), &pdf, None);
    }
}
