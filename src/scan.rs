//! High-level scan orchestration used by CLI, GUI, and tests.

use crate::atomic_write::{output_paths_alias, validate_output_leaf};
use crate::config::AppConfig;
use crate::core::{PipelinePrefs, Result, ScanError, ScanMode, ScanProgress, ScanRequest};
use crate::device::{
    open_device_with_policy, resolve_device_id, CancellationToken, DeviceOpenPolicy, DeviceSession,
};
use crate::export::{
    apply_export_profile, prepare_export_options, save_final_image_with_cancellation, ExportOptions,
};
use crate::imaging::save_raw_image_with_cancellation;
use crate::pipeline::{apply_pipeline, white_balance};
use std::path::PathBuf;

/// Typed scan arguments shared by each application entry point.
pub struct ScanToFileArgs {
    pub device: Option<String>,
    pub out: PathBuf,
    pub width: u32,
    pub height: u32,
    pub seed: u32,
    pub dpi: u32,
    pub mode: ScanMode,
    pub duplex: bool,
    pub pipeline: PipelinePrefs,
    pub invert_colors: bool,
    pub use_preview: bool,
    pub config: Option<AppConfig>,
    pub on_progress: Option<Box<dyn Fn(ScanProgress) + Send>>,
    /// When returns true, abort acquire/pipeline and call `session.cancel()`.
    pub cancel_check: Option<Box<dyn Fn() -> bool + Send>>,
    /// Optional pre-pipeline TIFF or image archive.
    pub raw_out: Option<PathBuf>,
}

impl Default for ScanToFileArgs {
    fn default() -> Self {
        Self {
            device: Some("mock".into()),
            out: PathBuf::from("scan.png"),
            width: 320,
            height: 240,
            seed: 1,
            dpi: 150,
            mode: ScanMode::Reflective,
            duplex: false,
            pipeline: PipelinePrefs::default(),
            invert_colors: false,
            use_preview: false,
            config: None,
            on_progress: None,
            cancel_check: None,
            raw_out: None,
        }
    }
}

/// Full orchestration path with optional progress callback and cancellation.
pub fn run_scan_to_file(args: ScanToFileArgs) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(
        args,
        &ExportOptions::default(),
        CancellationToken::new(),
    )
}

/// Full orchestration path bound to a shared cancellation signal.
pub fn run_scan_to_file_with_token(
    args: ScanToFileArgs,
    token: CancellationToken,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(args, &ExportOptions::default(), token)
}

/// Scan with runtime-only PDF/OCR/profile export controls.
pub fn run_scan_to_file_with_export_options(
    args: ScanToFileArgs,
    export: &ExportOptions,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(args, export, CancellationToken::new())
}

/// Scan with export controls and a token bound directly to the opened session.
pub fn run_scan_to_file_with_export_options_and_token(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token_and_policy(
        args,
        export,
        token,
        DeviceOpenPolicy::default(),
    )
}

/// Token-aware scan with explicit device-opening policy.
pub fn run_scan_to_file_with_export_options_and_token_and_policy(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<PathBuf> {
    // Validate output-only settings before device acquisition or any output write.
    let export = prepare_export_options(&args.out, export)?;
    let plan = prepare_scan(&args)?;
    if is_cancelled(&args, &token) {
        return Err(ScanError::Cancelled("scan cancelled".into()));
    }

    let dev_id = resolve_device_id(args.device.as_deref());
    report_progress(&args, "open", 0.05, &format!("opening {dev_id}"));

    let session = open_device_with_policy(&dev_id, policy)?;
    session.bind_cancellation(token.clone());
    let result = {
        let context = ScanContext::new(&args, dev_id, plan, &export, &token);
        context.run(&session)
    };
    session.close();
    result
}

struct ScanPlan {
    pipeline: PipelinePrefs,
    region: Option<crate::core::Rect>,
    extra_white_balance: bool,
}

fn prepare_scan(args: &ScanToFileArgs) -> Result<ScanPlan> {
    let cfg = args.config.clone().unwrap_or_default();
    // Fold invert into pipeline once — never apply invert twice (pipeline + flag).
    let mut pipeline = args.pipeline.clone();
    if args.invert_colors || cfg.invert_colors {
        pipeline.invert = true;
    }
    // Fold config white_balance only when not already requested on pipeline.
    let extra_white_balance = cfg.white_balance && !pipeline.white_balance;
    validate_scan_args(args, &pipeline)?;
    // A scan crop is a device acquisition region. Clear it from the
    // post-acquisition pipeline so hardware and simulated sources never crop
    // the same rectangle twice.
    let region = pipeline.crop.take();
    Ok(ScanPlan {
        pipeline,
        region,
        extra_white_balance,
    })
}

fn validate_scan_args(args: &ScanToFileArgs, pipeline: &PipelinePrefs) -> Result<()> {
    validate_dimensions(args)?;
    crate::core::validate_scan_dpi(args.dpi, args.dpi)?;
    validate_output_destinations(args)?;
    if args.duplex {
        return Err(ScanError::Invalid(
            "single-image scan cannot return both duplex sides; use batch --source adf --duplex with an even --pages value"
                .into(),
        ));
    }
    if let Some(crop) = pipeline.crop {
        validate_crop(args, crop)?;
    }
    Ok(())
}

fn validate_output_destinations(args: &ScanToFileArgs) -> Result<()> {
    validate_image_output_path(&args.out, "scan output")?;
    let Some(raw_out) = args.raw_out.as_ref() else {
        return Ok(());
    };
    let mut effective_raw_out = raw_out.clone();
    if effective_raw_out.extension().is_none() {
        effective_raw_out.set_extension("tif");
    }
    validate_image_output_path(&effective_raw_out, "raw output")?;
    if output_paths_alias(&args.out, &effective_raw_out)? {
        return Err(ScanError::Invalid(format!(
            "raw output aliases final output: {}",
            args.out.display()
        )));
    }
    Ok(())
}

fn validate_image_output_path(path: &std::path::Path, label: &str) -> Result<()> {
    validate_output_leaf(path, label)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid(format!("{label} has no supported extension")))?;
    if !crate::imaging::supported_extensions().contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

fn validate_dimensions(args: &ScanToFileArgs) -> Result<()> {
    if args.width == 0 || args.height == 0 {
        return Err(ScanError::Invalid(
            "width and height must be positive".into(),
        ));
    }
    crate::core::checked_image_len(
        args.width,
        args.height,
        crate::core::PixelFormat::Rgb8.bpp(),
    )?;
    Ok(())
}

fn validate_crop(args: &ScanToFileArgs, crop: crate::core::Rect) -> Result<()> {
    if crop_has_invalid_shape(crop) {
        return Err(ScanError::Invalid(
            "crop must be x,y,w,h with positive size".into(),
        ));
    }
    if crop_exceeds_scan_size(args, crop) {
        return Err(ScanError::Invalid(format!(
            "crop {},{},{},{} exceeds scan size {}x{}",
            crop.x, crop.y, crop.width, crop.height, args.width, args.height
        )));
    }
    Ok(())
}

fn crop_has_invalid_shape(crop: crate::core::Rect) -> bool {
    crop.x < 0 || crop.y < 0 || crop.width < 1 || crop.height < 1
}

fn crop_exceeds_scan_size(args: &ScanToFileArgs, crop: crate::core::Rect) -> bool {
    crop.x as u32 + crop.width > args.width || crop.y as u32 + crop.height > args.height
}

fn report_progress(args: &ScanToFileArgs, phase: &str, percent: f64, message: &str) {
    if let Some(ref callback) = args.on_progress {
        callback(ScanProgress::new(phase, percent, message));
    }
}

fn is_cancelled(args: &ScanToFileArgs, token: &CancellationToken) -> bool {
    token.is_cancelled()
        || args
            .cancel_check
            .as_ref()
            .map(|check| check())
            .unwrap_or(false)
}

struct ScanContext<'args> {
    args: &'args ScanToFileArgs,
    request: ScanRequest,
    extra_white_balance: bool,
    export: &'args crate::export::PreparedExportOptions,
    token: &'args CancellationToken,
}

impl<'args> ScanContext<'args> {
    fn new(
        args: &'args ScanToFileArgs,
        device_id: String,
        plan: ScanPlan,
        export: &'args crate::export::PreparedExportOptions,
        token: &'args CancellationToken,
    ) -> Self {
        Self {
            args,
            request: ScanRequest {
                device_id,
                width: args.width,
                height: args.height,
                dpi_x: args.dpi,
                dpi_y: args.dpi,
                mode: args.mode,
                duplex: args.duplex,
                region: plan.region,
                seed: args.seed,
                pipeline: plan.pipeline,
                ..ScanRequest::default()
            },
            extra_white_balance: plan.extra_white_balance,
            export,
            token,
        }
    }

    fn run(&self, session: &impl DeviceSession) -> Result<PathBuf> {
        self.cancel_if_requested(session)?;
        session.set_params(&self.request)?;
        report_progress(
            self.args,
            "acquire",
            0.35,
            if self.args.use_preview {
                "preview"
            } else {
                "scanning"
            },
        );
        self.cancel_if_requested(session)?;
        let mut image = if self.args.use_preview {
            session.preview(&self.request)?
        } else {
            session.scan(&self.request)?
        };
        if let Some(raw_out) = self.args.raw_out.as_ref() {
            save_raw_image_with_cancellation(
                raw_out,
                &image,
                Some(self.args.dpi),
                Some(self.token),
            )?;
        }
        self.cancel_if_requested(session)?;
        report_progress(self.args, "pipeline", 0.7, "pipeline");
        // Single invert application lives inside apply_pipeline via pipeline.invert.
        image = apply_pipeline(&image, &self.request.pipeline)?;
        if self.extra_white_balance {
            image = white_balance(&image)?;
        }
        // Profiles apply exactly once to the final post-pipeline pixels.
        image = apply_export_profile(&image, self.export)?;
        self.cancel_if_requested(session)?;
        report_progress(self.args, "save", 0.9, &self.args.out.display().to_string());
        let path = save_final_image_with_cancellation(
            &self.args.out,
            &image,
            Some(self.args.dpi),
            None,
            self.export,
            Some(self.token),
        )?;
        report_progress(self.args, "done", 1.0, &path.display().to_string());
        Ok(path)
    }

    fn cancel_if_requested(&self, session: &impl DeviceSession) -> Result<()> {
        if is_cancelled(self.args, self.token) {
            session.cancel();
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{PipelinePrefs, Rect};
    use crate::export::ExportOptions;
    use crate::imaging::load_image;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn run_scan_to_file_writes_nonempty_png() {
        let dir = std::env::temp_dir().join("open_scanline_scan_test");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("unit_scan.png");
        let path = run_scan_to_file(ScanToFileArgs {
            out: out.clone(),
            width: 32,
            height: 24,
            seed: 3,
            ..ScanToFileArgs::default()
        })
        .expect("scan");
        assert!(path.is_file());
        let meta = std::fs::metadata(&path).unwrap();
        assert!(meta.len() > 0);
        let head = std::fs::read(&path).unwrap();
        assert!(head.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn malformed_export_profile_rejects_before_scan_or_output() {
        let dir = std::env::temp_dir().join("open_scanline_scan_bad_export_profile");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("unchanged.png");
        let profile = dir.join("bad-profile.json");
        std::fs::write(&out, b"existing output").unwrap();
        std::fs::write(&profile, "{bad json").unwrap();
        let error = run_scan_to_file_with_export_options(
            ScanToFileArgs {
                out: out.clone(),
                ..ScanToFileArgs::default()
            },
            &ExportOptions {
                scanner_profile: Some(profile),
                ..ExportOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("invalid scanner profile"));
        assert_eq!(std::fs::read(out).unwrap(), b"existing output");
    }

    #[test]
    fn invalid_effective_pdf_password_rejects_before_device_open_or_output() {
        let dir = std::env::temp_dir().join("open_scanline_scan_bad_pdf_password");
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("never-created.pdf");
        let error = run_scan_to_file_with_export_options(
            ScanToFileArgs {
                device: Some("not-a-device".into()),
                out: out.clone(),
                ..ScanToFileArgs::default()
            },
            &ExportOptions {
                // SASLprep maps SOFT HYPHEN to nothing, which must not become
                // an effectively empty PDF password after acquisition.
                pdf_password: Some("\u{00ad}".into()),
                ..ExportOptions::default()
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("empty after SASLprep"));
        assert!(!out.exists());
        assert!(!dir.exists());
    }

    #[test]
    fn scan_crop_is_forwarded_as_one_acquisition_region() {
        let directory = std::env::temp_dir().join("open_scanline_scan_region");
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("region.png");
        run_scan_to_file(ScanToFileArgs {
            out: output.clone(),
            width: 80,
            height: 40,
            pipeline: PipelinePrefs {
                crop: Some(Rect::new(10, 5, 30, 20)),
                ..PipelinePrefs::default()
            },
            ..ScanToFileArgs::default()
        })
        .unwrap();

        let image = load_image(output).unwrap();
        assert_eq!((image.width, image.height), (30, 20));
        assert_eq!(image.data[0], (10 * 255 / 79) as u8);
        assert_eq!(image.data[1], (5 * 255 / 39) as u8);
    }

    #[test]
    fn invalid_dimensions_and_crop_fail_before_open_or_progress() {
        let progress = Arc::new(Mutex::new(Vec::new()));
        let width_progress = progress.clone();
        let width_error = run_scan_to_file(ScanToFileArgs {
            device: Some("not-a-device".into()),
            width: 0,
            on_progress: Some(Box::new(move |event| {
                width_progress.lock().unwrap().push(event.phase);
            })),
            ..ScanToFileArgs::default()
        })
        .expect_err("zero width must fail validation before opening a device");
        assert!(matches!(
            width_error,
            ScanError::Invalid(message) if message == "width and height must be positive"
        ));
        assert!(progress.lock().unwrap().is_empty());

        let duplex_progress = progress.clone();
        let duplex_output = std::env::temp_dir().join("open_scanline_invalid_single_duplex.png");
        let _ = std::fs::remove_file(&duplex_output);
        let duplex_error = run_scan_to_file(ScanToFileArgs {
            device: Some("not-a-device".into()),
            out: duplex_output.clone(),
            mode: ScanMode::Document,
            duplex: true,
            on_progress: Some(Box::new(move |event| {
                duplex_progress.lock().unwrap().push(event.phase);
            })),
            ..ScanToFileArgs::default()
        })
        .expect_err("single-image duplex must fail before opening a device");
        assert!(matches!(
            duplex_error,
            ScanError::Invalid(message) if message.contains("batch --source adf --duplex")
        ));
        assert!(progress.lock().unwrap().is_empty());
        assert!(!duplex_output.exists());

        let crop_progress = progress.clone();
        let crop_error = run_scan_to_file(ScanToFileArgs {
            device: Some("not-a-device".into()),
            width: 32,
            height: 24,
            pipeline: PipelinePrefs {
                crop: Some(Rect::new(24, 0, 16, 12)),
                ..PipelinePrefs::default()
            },
            on_progress: Some(Box::new(move |event| {
                crop_progress.lock().unwrap().push(event.phase);
            })),
            ..ScanToFileArgs::default()
        })
        .expect_err("out-of-bounds crop must fail validation before opening a device");
        assert!(matches!(
            crop_error,
            ScanError::Invalid(message)
                if message == "crop 24,0,16,12 exceeds scan size 32x24"
        ));
        assert!(progress.lock().unwrap().is_empty());
    }

    #[test]
    fn progress_and_cancellation_checks_follow_scan_order() {
        let directory = std::env::temp_dir().join("open_scanline_scan_progress");
        std::fs::create_dir_all(&directory).unwrap();
        let out = directory.join("ordered.png");
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_events = progress.clone();
        let cancellation_checks = Arc::new(AtomicUsize::new(0));
        let checks = cancellation_checks.clone();

        run_scan_to_file(ScanToFileArgs {
            out: out.clone(),
            width: 16,
            height: 12,
            on_progress: Some(Box::new(move |event| {
                progress_events
                    .lock()
                    .unwrap()
                    .push((event.phase, event.percent, event.message));
            })),
            cancel_check: Some(Box::new(move || {
                checks.fetch_add(1, Ordering::SeqCst);
                false
            })),
            ..ScanToFileArgs::default()
        })
        .expect("scan succeeds");

        assert_eq!(cancellation_checks.load(Ordering::SeqCst), 5);
        assert_eq!(
            *progress.lock().unwrap(),
            vec![
                ("open".into(), 0.05, "opening mock".into()),
                ("acquire".into(), 0.35, "scanning".into()),
                ("pipeline".into(), 0.7, "pipeline".into()),
                ("save".into(), 0.9, out.display().to_string()),
                ("done".into(), 1.0, out.display().to_string()),
            ]
        );

        let cancellation_progress = Arc::new(Mutex::new(Vec::new()));
        let cancellation_events = cancellation_progress.clone();
        let cancellation_checks = Arc::new(AtomicUsize::new(0));
        let checks = cancellation_checks.clone();
        let error = run_scan_to_file(ScanToFileArgs {
            out,
            on_progress: Some(Box::new(move |event| {
                cancellation_events.lock().unwrap().push(event.phase);
            })),
            cancel_check: Some(Box::new(move || checks.fetch_add(1, Ordering::SeqCst) == 1)),
            ..ScanToFileArgs::default()
        })
        .expect_err("second cancellation check runs after opening the session");
        assert!(matches!(
            error,
            ScanError::Cancelled(message) if message == "scan cancelled"
        ));
        assert_eq!(cancellation_checks.load(Ordering::SeqCst), 2);
        assert_eq!(*cancellation_progress.lock().unwrap(), vec!["open"]);
    }

    /// End-to-end: invert must change pixels vs same seed without invert.
    /// Catches double-invert (identity) regression on the shared scan path.
    #[test]
    fn scan_invert_changes_pixels_vs_same_seed() {
        let dir = std::env::temp_dir().join("open_scanline_scan_invert");
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("no_inv.png");
        let inverted = dir.join("with_inv.png");
        let seed = 42u32;
        let w = 48u32;
        let h = 36u32;

        run_scan_to_file(ScanToFileArgs {
            out: plain.clone(),
            width: w,
            height: h,
            seed,
            ..ScanToFileArgs::default()
        })
        .expect("plain scan");

        // Path A: invert only via pipeline.invert
        let prefs = PipelinePrefs {
            invert: true,
            ..PipelinePrefs::default()
        };
        run_scan_to_file(ScanToFileArgs {
            out: inverted.clone(),
            width: w,
            height: h,
            seed,
            pipeline: prefs,
            ..ScanToFileArgs::default()
        })
        .expect("invert scan");

        let img_plain = load_image(&plain).expect("load plain");
        let img_inv = load_image(&inverted).expect("load inverted");
        assert_eq!(img_plain.width, img_inv.width);
        assert_eq!(img_plain.height, img_inv.height);
        assert_ne!(
            img_plain.data, img_inv.data,
            "scan with invert must change pixel data vs same seed without invert"
        );
        // Double-invert regression: first pixel must be true invert of plain
        assert_eq!(
            img_inv.data[0],
            255 - img_plain.data[0],
            "single invert: first channel should be 255-plain"
        );

        // Path B: invert only via invert_colors flag (must not double with pipeline)
        let inv_flag = dir.join("with_inv_flag.png");
        let path_flag = run_scan_to_file(ScanToFileArgs {
            device: Some("mock".into()),
            out: inv_flag.clone(),
            width: w,
            height: h,
            seed,
            dpi: 150,
            mode: ScanMode::Reflective,
            duplex: false,
            pipeline: PipelinePrefs::default(),
            invert_colors: true,
            use_preview: false,
            config: None,
            on_progress: None,
            cancel_check: None,
            raw_out: None,
        })
        .expect("invert_colors flag scan");
        let img_flag = load_image(&path_flag).expect("load flag invert");
        assert_eq!(
            img_flag.data, img_inv.data,
            "invert_colors flag must match pipeline.invert (single apply)"
        );

        // Path C: both set → still single invert (not identity)
        let both = dir.join("with_inv_both.png");
        let prefs_both = PipelinePrefs {
            invert: true,
            ..PipelinePrefs::default()
        };
        let path_both = run_scan_to_file(ScanToFileArgs {
            device: Some("mock".into()),
            out: both.clone(),
            width: w,
            height: h,
            seed,
            dpi: 150,
            mode: ScanMode::Reflective,
            duplex: false,
            pipeline: prefs_both,
            invert_colors: true,
            use_preview: false,
            config: None,
            on_progress: None,
            cancel_check: None,
            raw_out: None,
        })
        .expect("both invert sources");
        let img_both = load_image(&path_both).expect("load both");
        assert_ne!(
            img_both.data, img_plain.data,
            "pipeline.invert + invert_colors must NOT double-invert to identity"
        );
        assert_eq!(
            img_both.data, img_inv.data,
            "both sources still single invert"
        );
    }

    #[test]
    fn token_aware_orchestration_stops_before_opening_a_session() {
        let token = CancellationToken::new();
        token.cancel();
        let output = std::env::temp_dir().join("open_scanline_token_cancelled.png");
        let _ = std::fs::remove_file(&output);

        let error = run_scan_to_file_with_token(
            ScanToFileArgs {
                out: output.clone(),
                ..ScanToFileArgs::default()
            },
            token,
        )
        .unwrap_err();

        assert!(matches!(error, ScanError::Cancelled(_)));
        assert!(!output.exists());
    }

    #[test]
    fn low_dpi_is_rejected_before_output_or_device_open() {
        let directory = std::env::temp_dir().join("open_scanline_scan_low_dpi");
        let _ = std::fs::remove_dir_all(&directory);
        let output = directory.join("scan.png");
        let error = run_scan_to_file(ScanToFileArgs {
            device: Some("missing-backend:fixture".into()),
            out: output.clone(),
            dpi: crate::core::MIN_SCAN_DPI - 1,
            ..ScanToFileArgs::default()
        })
        .unwrap_err();

        assert!(error.to_string().contains("at least 50 dpi"));
        assert!(!output.exists());
        assert!(!directory.exists());
    }

    #[test]
    fn raw_output_alias_is_rejected_before_device_open_or_overwrite() {
        let directory = std::env::temp_dir().join("open_scanline_scan_raw_alias");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let output = directory.join("archive.tif");
        std::fs::write(&output, b"existing final output").unwrap();

        let error = run_scan_to_file(ScanToFileArgs {
            device: Some("missing-backend:fixture".into()),
            out: output.clone(),
            // Extensionless raw outputs are materialized as TIFF, so this is
            // the same effective destination as `out`.
            raw_out: Some(directory.join("archive")),
            ..ScanToFileArgs::default()
        })
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("raw output aliases final output"));
        assert_eq!(std::fs::read(output).unwrap(), b"existing final output");
    }

    #[test]
    fn unsupported_final_and_raw_formats_fail_before_device_open() {
        for (out, raw_out, expected) in [
            (PathBuf::from("scan.unsupported"), None, "scan output"),
            (
                PathBuf::from("scan.png"),
                Some(PathBuf::from("raw.unsupported")),
                "raw output",
            ),
        ] {
            let error = run_scan_to_file(ScanToFileArgs {
                device: Some("missing-backend:fixture".into()),
                out,
                raw_out,
                ..ScanToFileArgs::default()
            })
            .unwrap_err();
            assert!(error.to_string().contains(expected));
            assert!(!error.to_string().contains("unknown device"));
        }
    }

    #[test]
    fn directory_destination_fails_before_device_open() {
        let directory = std::env::temp_dir().join("open_scanline_scan_directory.png");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let error = run_scan_to_file(ScanToFileArgs {
            device: Some("missing-backend:fixture".into()),
            out: directory,
            ..ScanToFileArgs::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("regular file"));
        assert!(!error.to_string().contains("unknown device"));
    }
}
