//! External consumer characterization for the intentionally preserved facades.
//!
//! Keep this focused on public names, constructors, and signatures.  Runtime
//! behavior is covered elsewhere; this file should remain valid across an
//! change to the internal module layout.

use open_scanline::batch::{self, BatchScanArgs};
use open_scanline::core::{ImageBuffer, PixelFormat, Result, ScanMode, ScanRequest};
use open_scanline::device::{
    AnySession, BackendInfo, CancellationToken, DeviceInfo, DeviceMaintenanceCapabilities,
    DeviceOpenPolicy, DeviceSession, FileDeviceSession, FocusCapability, MaintenanceAvailability,
    MockDevice, MockDeviceSession, ScanPagesEnd, ScanPagesResult,
};
use open_scanline::export::{ExportOptions, OcrEngine};
use open_scanline::imaging::{self, PdfOptions};
use open_scanline::ml::{
    OnnxInferenceOptions, OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
    OnnxRuntime,
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

fn assert_public_device_contract() {
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
    let simulated = DeviceMaintenanceCapabilities::simulated_point_focus();
    assert!(matches!(
        &simulated.calibration,
        MaintenanceAvailability::Simulated
    ));
    assert!(matches!(&simulated.focus, FocusCapability::Point { .. }));
    assert_eq!(
        ScanPagesResult::limit_reached(2).end,
        ScanPagesEnd::LimitReached
    );
    assert_eq!(
        ScanPagesResult::feeder_exhausted(1).end,
        ScanPagesEnd::FeederExhausted
    );
}

fn public_image_and_request() -> (ImageBuffer, ScanRequest) {
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![1, 2, 3])
        .expect("public image constructor accepts packed RGB data");
    let request = ScanRequest {
        mode: ScanMode::Reflective,
        ..ScanRequest::default()
    };
    assert_eq!(request.dpi_x, 150);
    (image, request)
}

fn public_export_and_pdf_options() -> (ExportOptions, PdfOptions) {
    let export = ExportOptions {
        pdf_password: None,
        searchable_pdf: true,
        ocr_language: "eng".into(),
        ocr_engine: OcrEngine::Offline,
        scanner_profile: None,
    };
    assert!(export.searchable_pdf);
    let _: OcrEngine = OcrEngine::Ocrs;
    let pdf = PdfOptions {
        dpi: 150,
        title: "compatibility".into(),
        password: None,
        searchable_pages: None,
    };
    assert_eq!(pdf.dpi, 150);
    (export, pdf)
}

fn assert_public_ml_contract(image: &ImageBuffer) {
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
    let ml_info = open_scanline::ml::ml_module_info();
    assert_eq!(ml_info["explicit_worker_runtime"], true);
    assert_eq!(ml_info["automatic_worker_discovery"], false);
    let runtime: Result<OnnxRuntime> = OnnxRuntime::from_worker(Path::new("open-scanline"));
    if false {
        let runtime = runtime.expect("only type-check the public runtime API");
        let _: Result<OnnxReport> = runtime.run(image, Path::new("model.onnx"));
        let _: Result<OnnxReport> =
            runtime.run_with_options(image, Path::new("model.onnx"), &onnx_options);
    }
}

fn assert_public_request_types() {
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
}

fn assert_public_operation_signatures() {
    assert_public_scan_signatures();
    assert_public_batch_signatures();
    assert_public_process_signatures();
}

fn assert_public_scan_signatures() {
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
}

fn assert_public_batch_signatures() {
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
}

fn assert_public_process_signatures() {
    let _: fn(&ProcessOptions) -> Result<PathBuf> = process::process_image_file;
    let _: fn(&ProcessOptions, CancellationToken) -> Result<PathBuf> =
        process::process_image_file_with_token;
    let _: fn(&ProcessOptions, &ExportOptions) -> Result<PathBuf> =
        process::process_image_file_with_export_options;
    let _: fn(&ProcessOptions, &ExportOptions, CancellationToken) -> Result<PathBuf> =
        process::process_image_file_with_export_options_and_token;
}

fn assert_public_adapter_contracts(request: &ScanRequest) {
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
            request,
            Path::new("scan.png")
        )
        .program,
        "scanimage"
    );
    assert!(
        wia::wia_transfer_command("compat", request, Path::new("scan.png"))
            .args
            .iter()
            .any(|argument| argument == "-Command")
    );
}

fn assert_public_pdf_contract(image: ImageBuffer, pdf: &PdfOptions) {
    // These calls prove the public, generic PDF API without producing files.
    if false {
        let pages = vec![PathBuf::from("page.png")];
        let _ = imaging::save_pdf_with_options(Path::new("out.pdf"), &[image], pdf);
        let _ = imaging::save_pdf_with_options_and_cancellation(
            Path::new("out.pdf"),
            &[],
            pdf,
            Some(&CancellationToken::new()),
        );
        let _ = imaging::save_multipage_pdf(&pages, Path::new("out.pdf"), pdf);
        let _ =
            imaging::save_multipage_pdf_with_cancellation(&pages, Path::new("out.pdf"), pdf, None);
    }
}

#[test]
fn preserved_public_facades_remain_usable_by_external_callers() {
    assert_public_device_contract();
    let (image, request) = public_image_and_request();
    let (_export, pdf) = public_export_and_pdf_options();
    assert_public_ml_contract(&image);
    assert_public_request_types();
    assert_public_operation_signatures();
    assert_public_adapter_contracts(&request);
    assert_public_pdf_contract(image, &pdf);
}

fn assert_film_facade() {
    use open_scanline::film::{convert_film, get_film_profile, list_film_profiles, FilmProfile};
    let _ = convert_film;
    let _ = get_film_profile;
    let profiles: Vec<FilmProfile> = list_film_profiles();
    assert!(!profiles.is_empty());
    assert!(!profiles[0].id.is_empty());
}

fn assert_icc_facade() {
    use open_scanline::icc::{
        apply_scanner_profile, build_icc_bytes, it8_reference_patches, load_scanner_profile,
        make_it8_target_image, profile_scanner_it8, save_profile_json, validate_scanner_profile,
    };
    let _ = apply_scanner_profile;
    let _ = build_icc_bytes;
    let _ = it8_reference_patches;
    let _ = make_it8_target_image;
    let _ = profile_scanner_it8;
    let _ = validate_scanner_profile;
    if false {
        // `impl AsRef<Path>` parameters cannot be named as bare fn items; type-check
        // the call shape instead without touching the filesystem.
        let _ = load_scanner_profile(Path::new("scanner-profile.json"));
        let _ = save_profile_json(Path::new("scanner-profile.json"), &serde_json::Value::Null);
    }
}

fn assert_manufacturers_facade() {
    use open_scanline::manufacturers::{
        format_manufacturers_text, list_manufacturers, manufacturer_support_summary,
        resolve_manufacturer, ManufacturerEntry,
    };
    let _ = format_manufacturers_text;
    let entries: Vec<ManufacturerEntry> = list_manufacturers();
    assert!(!entries.is_empty());
    assert!(manufacturer_support_summary().is_object());
    let _ = resolve_manufacturer("definitely-not-a-manufacturer");
}

fn assert_i18n_facade() {
    use open_scanline::i18n::{
        available_languages, language_name, translate, translate_args, validate_catalogs,
        Translator,
    };
    let languages = available_languages();
    assert!(languages.iter().any(|code| code == "en"));
    assert_eq!(language_name("en"), "English");
    let mut translator = Translator::new("en");
    assert_eq!(translator.language(), "en");
    assert_eq!(translator.t("app.name"), "Open Scanline");
    let _ = translator.t_args("app.name", &[]);
    assert_eq!(translate("en", "app.name"), "Open Scanline");
    let _ = translate_args("en", "app.name", &[]);
    let _ = translator.set_language("de");
    let _ = validate_catalogs();
}

fn assert_features_facade() {
    use open_scanline::features::feature_matrix;
    assert!(feature_matrix().is_object());
}

fn assert_platform_facade() {
    use open_scanline::platform::{app_name, cache_dir, config_dir, data_dir, platform_summary};
    assert_eq!(app_name(), open_scanline::APP_NAME);
    let _ = config_dir();
    let _ = data_dir();
    let _ = cache_dir();
    assert!(platform_summary().is_object());
}

fn assert_ocr_facade() {
    use open_scanline::ocr::{
        ocr_file, ocr_file_with_cancellation, ocr_file_with_engine_with_cancellation, ocr_image,
        ocr_image_offline, ocr_image_tesseract, ocr_image_tesseract_with_cancellation,
        ocr_image_with_cancellation, ocr_image_with_engine_with_cancellation, ocr_module_info,
        tesseract_available, OcrResult, OFFLINE_OCR_ENGINE,
    };
    assert_eq!(OFFLINE_OCR_ENGINE, "offline-template");
    let _ = tesseract_available();
    assert!(ocr_module_info().is_object());
    let _ = ocr_image_offline;
    let _ = ocr_image_tesseract;
    let _ = ocr_image_tesseract_with_cancellation;
    let _ = ocr_image;
    let _ = ocr_image_with_cancellation;
    let _ = ocr_image_with_engine_with_cancellation;
    if false {
        // `ocr_file*` take `impl AsRef<Path>`; type-check without external OCR I/O.
        let _: Result<OcrResult> = ocr_file(Path::new("scan.png"), "eng", true);
        let _: Result<OcrResult> = ocr_file_with_cancellation(
            Path::new("scan.png"),
            "eng",
            true,
            CancellationToken::new(),
        );
        let _: Result<OcrResult> = ocr_file_with_engine_with_cancellation(
            Path::new("scan.png"),
            "eng",
            OcrEngine::Offline,
            CancellationToken::new(),
        );
    }
}

fn assert_escl_facade() {
    use open_scanline::escl::{
        available, backend_info, discover_devices, list_devices, list_escl_devices_safe,
        list_escl_devices_safe_with_cancellation, open, open_explicit_id, open_unlisted_endpoint,
        open_with_cancellation, parse_job_id, refresh_devices, refresh_devices_with_cancellation,
        EsclDeviceSession,
    };
    assert_device_session::<EsclDeviceSession>();
    let _ = available();
    assert!(!backend_info().id.is_empty());
    assert_eq!(parse_job_id("", b""), None);
    // Discovery/opening entry points would touch the network; keep them as
    // signature-only references so this contract stays network-free.
    let _ = discover_devices;
    let _ = list_devices;
    let _ = list_escl_devices_safe;
    let _ = list_escl_devices_safe_with_cancellation;
    let _ = refresh_devices;
    let _ = refresh_devices_with_cancellation;
    let _ = open;
    let _ = open_explicit_id;
    let _ = open_unlisted_endpoint;
    let _ = open_with_cancellation;
}

fn assert_plugin_facade() {
    use open_scanline::plugin::{
        plugin_status, plugin_status_with_cancellation, run_plugin_mode, run_plugin_mode_with_token,
    };
    let _ = run_plugin_mode;
    let _ = run_plugin_mode_with_token;
    if false {
        // Calling these performs live device discovery; type-check them only.
        let _: serde_json::Value = plugin_status(None);
        let _: serde_json::Value = plugin_status_with_cancellation(None, None);
    }
}

fn assert_twain_facade() {
    use open_scanline::twain::{launch_plugin_host, resolve_host_command, twain_shim_info};
    let info = twain_shim_info();
    assert_eq!(info["module"], "open_scanline::twain");
    assert_eq!(info["ships_native_ds"], false);
    assert_eq!(
        resolve_host_command("plugin", None, None),
        vec!["plugin".to_string()]
    );
    // Launching spawns or runs a full plugin session; keep this a signature check.
    let _ = launch_plugin_host;
}

fn assert_config_facade() {
    use open_scanline::config::{
        config_from_value, default_config_path, format_curve_points, is_banned_key, load_config,
        parse_curve_points, save_config, strip_banned, validate_hue, validate_output_name,
        validate_saturation, AppConfig, MAX_OUTPUT_NAME_BYTES,
    };
    assert_eq!(MAX_OUTPUT_NAME_BYTES, 128);
    let _ = default_config_path();
    assert!(is_banned_key("license_key"));
    assert!(!is_banned_key("default_dpi"));
    assert_eq!(validate_output_name("scan").unwrap(), "scan");
    assert_eq!(validate_hue(10.0).unwrap(), 10.0);
    assert_eq!(validate_saturation(10.0).unwrap(), 10.0);
    assert_eq!(format_curve_points(None), "");
    assert!(parse_curve_points("").unwrap().is_none());
    let config: AppConfig =
        config_from_value(serde_json::to_value(AppConfig::default()).unwrap()).unwrap();
    assert_eq!(config, AppConfig::default());
    assert!(strip_banned(&serde_json::Value::Null).is_null());
    let _ = load_config;
    if false {
        // `save_config` takes `impl AsRef<Path>`; type-check without file I/O.
        let _ = save_config(&AppConfig::default(), Path::new("config.json"));
    }
}

fn assert_cli_facade() {
    let _: fn(&[String]) -> i32 = open_scanline::cli::run;
}

#[cfg(feature = "gui")]
fn assert_gui_facade() {
    use open_scanline::gui::{
        normalize_image_ext, parse_crop_string, preview_texture_needs_reload, run_gui,
    };
    assert_eq!(normalize_image_ext("JPEG", "png"), "jpg");
    assert_eq!(parse_crop_string("").unwrap(), None);
    assert!(!preview_texture_needs_reload(None, None, None));
    // Launching opens a native window; keep this a signature-only reference.
    let _ = run_gui;
}

#[test]
fn additional_compatibility_facades_expose_their_documented_entry_points() {
    assert_film_facade();
    assert_icc_facade();
    assert_manufacturers_facade();
    assert_i18n_facade();
    assert_features_facade();
    assert_platform_facade();
    assert_ocr_facade();
    assert_escl_facade();
    assert_plugin_facade();
    assert_twain_facade();
    assert_config_facade();
    assert_cli_facade();
    #[cfg(feature = "gui")]
    assert_gui_facade();
}
