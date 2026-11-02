//! High-level scan orchestration used by CLI, GUI, and tests.

use crate::domain::acquisition::{
    validate_scan_dpi, DeviceOpenPolicy, ScanMode, ScanProgress, ScanRequest,
};
use crate::domain::export::ExportOptions;
use crate::domain::image::{checked_image_len, ImageBuffer, PixelFormat, Rect};
use crate::domain::processing::PipelinePrefs;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::DeviceSession;
use crate::infrastructure::config::AppConfig;
use crate::infrastructure::runtime::atomic_publish::ensure_output_absent;
use crate::operation::CancellationToken;
use crate::workflows::process::{process_and_publish_page, PageWorkflowRequest};
use crate::workflows::publication::{
    prepare_export_options, validate_supported_output_path, PreparedExportOptions,
};
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
    /// Replace existing output files; when false an existing destination is
    /// rejected before acquisition.
    pub overwrite: bool,
}

impl Default for ScanToFileArgs {
    fn default() -> Self {
        Self {
            device: Some("mock".into()),
            out: PathBuf::from("scan.png"),
            // Zero is the device maximum; the mock renders it as 320x240.
            width: 0,
            height: 0,
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
            overwrite: false,
        }
    }
}

/// Runtime controls for the canonical capture entry.
#[derive(Default)]
pub struct CaptureOptions {
    pub export: ExportOptions,
    pub cancellation: CancellationToken,
    pub policy: DeviceOpenPolicy,
}

/// Scan using the native acquisition and media adapters.
pub fn run_scan_to_file(args: ScanToFileArgs, options: CaptureOptions) -> Result<PathBuf> {
    let CaptureOptions {
        export,
        cancellation: token,
        policy,
    } = options;
    // Validate output-only settings before device acquisition or any output write.
    let export = prepare_export_options(&args.out, &export)?;
    let plan = prepare_scan(&args)?;
    if is_cancelled(&args, &token) {
        return Err(ScanError::Cancelled("scan cancelled".into()));
    }

    let dev_id = crate::infrastructure::acquisition::resolve_device_id(args.device.as_deref());
    report_progress(&args, "open", 0.05, &format!("opening {dev_id}"));

    let session = crate::infrastructure::acquisition::open_device_with_policy(&dev_id, policy)?;
    session.bind_cancellation(token.clone());
    let result = {
        let context = ScanContext::new(&args, dev_id, plan, &export, &token);
        context.run(&session)
    };
    session.close();
    result
}

/// Scan with runtime-only PDF/OCR/profile export controls.
struct ScanPlan {
    pipeline: PipelinePrefs,
    region: Option<Rect>,
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
    validate_scan_dpi(args.dpi, args.dpi)?;
    validate_output_destinations(args)?;
    if args.duplex {
        return Err(ScanError::Invalid(
            "single-image scan cannot return both duplex sides; use batch --source adf --duplex with an even --pages value"
                .into(),
        ));
    }
    if let Some(crop) = pipeline.crop {
        validate_crop(args.width, args.height, crop)?;
    }
    validate_film_type(pipeline)
}

/// Reject an unknown film type before the device is opened.
pub(super) fn validate_film_type(pipeline: &PipelinePrefs) -> Result<()> {
    let film = pipeline.film_type.as_deref().map(str::trim);
    match film {
        Some(film)
            if !film.is_empty()
                && !film.eq_ignore_ascii_case("off")
                && !film.eq_ignore_ascii_case("none") =>
        {
            crate::domain::processing::get_film_profile(film).map(|_| ())
        }
        _ => Ok(()),
    }
}

fn validate_output_destinations(args: &ScanToFileArgs) -> Result<()> {
    validate_supported_output_path(&args.out, "scan output")?;
    ensure_output_absent(&args.out, args.overwrite)?;
    let Some(raw_out) = args.raw_out.as_ref() else {
        return Ok(());
    };
    let mut effective_raw_out = raw_out.clone();
    if effective_raw_out.extension().is_none() {
        effective_raw_out.set_extension("tif");
    }
    validate_supported_output_path(&effective_raw_out, "raw output")?;
    ensure_output_absent(&effective_raw_out, args.overwrite)?;
    if crate::infrastructure::runtime::atomic_publish::output_paths_alias(
        &args.out,
        &effective_raw_out,
    )? {
        return Err(ScanError::Invalid(format!(
            "raw output aliases final output: {}",
            args.out.display()
        )));
    }
    Ok(())
}

fn validate_dimensions(args: &ScanToFileArgs) -> Result<()> {
    // Zero means the device maximum; only explicit axes are bounded.
    checked_image_len(
        args.width.max(1),
        args.height.max(1),
        PixelFormat::Rgb8.bpp(),
    )?;
    Ok(())
}

pub(super) fn validate_crop(width: u32, height: u32, crop: Rect) -> Result<()> {
    if crop_has_invalid_shape(crop) {
        return Err(ScanError::Invalid(
            "crop must be x,y,w,h with positive size".into(),
        ));
    }
    if crop_exceeds_scan_size(width, height, crop) {
        return Err(ScanError::Invalid(format!(
            "crop {},{},{},{} exceeds scan size {}x{}",
            crop.x, crop.y, crop.width, crop.height, width, height
        )));
    }
    Ok(())
}

fn crop_has_invalid_shape(crop: Rect) -> bool {
    crop.x < 0 || crop.y < 0 || crop.width < 1 || crop.height < 1
}

fn crop_exceeds_scan_size(width: u32, height: u32, crop: Rect) -> bool {
    // An axis left to the device has no known size to check against.
    // Widened to u64 so a huge crop cannot overflow the sum.
    (width > 0 && crop.x as u64 + crop.width as u64 > width as u64)
        || (height > 0 && crop.y as u64 + crop.height as u64 > height as u64)
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
    export: &'args PreparedExportOptions,
    token: &'args CancellationToken,
}

impl<'args> ScanContext<'args> {
    fn new(
        args: &'args ScanToFileArgs,
        device_id: String,
        plan: ScanPlan,
        export: &'args PreparedExportOptions,
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

    fn run(&self, session: &(impl DeviceSession + ?Sized)) -> Result<PathBuf> {
        let image = self.acquire_image(session)?;
        let path = self.publish_image(image, session)?;
        report_progress(self.args, "done", 1.0, &path.display().to_string());
        Ok(path)
    }

    fn acquire_image(&self, session: &(impl DeviceSession + ?Sized)) -> Result<ImageBuffer> {
        self.cancel_if_requested(session)?;
        session.set_params(&self.request)?;
        self.report_acquisition_start();
        self.cancel_if_requested(session)?;
        self.acquire_from_session(session)
    }

    fn report_acquisition_start(&self) {
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
    }

    fn acquire_from_session(&self, session: &(impl DeviceSession + ?Sized)) -> Result<ImageBuffer> {
        let acquired = if self.args.use_preview {
            session.preview(&self.request)
        } else {
            session.scan(&self.request)
        };
        self.check_region_fits(acquired)
    }

    /// `crop_exceeds_scan_size` cannot check an axis left to the device
    /// maximum (0). The device applies the region itself, so name the crop and
    /// the requested size when it rejects the region as out of bounds.
    fn check_region_fits(&self, acquired: Result<ImageBuffer>) -> Result<ImageBuffer> {
        let Some(region) = self.request.region else {
            return acquired;
        };
        // A device rounds the region to its own geometry, so a slightly
        // smaller image is not an error; only an explicit rejection is reworded.
        match acquired {
            Err(ScanError::Invalid(message)) if message == "crop region out of bounds" => {
                Err(ScanError::Invalid(format!(
                    "crop {},{},{},{} exceeds the scan size delivered by the device (requested {}x{}; 0 means the device maximum)",
                    region.x, region.y, region.width, region.height, self.args.width, self.args.height
                )))
            }
            other => other,
        }
    }

    fn publish_image(
        &self,
        image: ImageBuffer,
        session: &(impl DeviceSession + ?Sized),
    ) -> Result<PathBuf> {
        if let Some(raw_out) = self.args.raw_out.as_ref() {
            let raw_path = crate::infrastructure::media::save_raw_image_with_cancellation(
                raw_out,
                &image,
                Some(self.args.dpi),
                Some(self.token),
            )?;
            report_progress(
                self.args,
                "raw-saved",
                0.55,
                &raw_path.display().to_string(),
            );
        }
        self.cancel_if_requested(session)?;
        report_progress(self.args, "pipeline", 0.7, "pipeline");
        self.cancel_if_requested(session)?;
        report_progress(self.args, "save", 0.9, &self.args.out.display().to_string());
        process_and_publish_page(
            image,
            PageWorkflowRequest {
                destination: &self.args.out,
                pipeline: &self.request.pipeline,
                extra_white_balance: self.extra_white_balance,
                dpi: Some(self.args.dpi),
                quality: None,
                export: self.export,
                cancellation: Some(self.token),
            },
        )
        .map(|result| result.path)
    }

    fn cancel_if_requested(&self, session: &(impl DeviceSession + ?Sized)) -> Result<()> {
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

    #[test]
    fn crop_bounds_check_does_not_overflow() {
        let crop = Rect::new(i32::MAX, 0, u32::MAX, 1);
        assert!(validate_crop(100, 100, crop).is_err());
        assert!(validate_crop(100, 100, Rect::new(10, 10, 90, 90)).is_ok());
    }
}
