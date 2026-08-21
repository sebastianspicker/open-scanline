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
