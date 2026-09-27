use super::{parse_curves, parse_hue, parse_saturation, CurvePoints, OcrEngineArg, ScanSource};
use clap::Args;
use std::path::PathBuf;

#[derive(Debug, Default, Args)]
pub(in crate::inbound::cli) struct ExportOptions {
    /// Add an OCR text layer when exporting to PDF
    #[arg(long = "pdf-searchable", action = clap::ArgAction::SetTrue)]
    pub(in crate::inbound::cli) pdf_searchable: bool,
    /// UNSAFE/DEPRECATED: literal PDF password; requires explicit opt-in
    #[arg(
        long = "pdf-password",
        conflicts_with = "pdf_password_file",
        requires = "allow_insecure_password_argv"
    )]
    pub(in crate::inbound::cli) pdf_password: Option<String>,
    /// Read a PDF password from PATH, or - for standard input
    #[arg(long = "pdf-password-file", conflicts_with = "pdf_password")]
    pub(in crate::inbound::cli) pdf_password_file: Option<PathBuf>,
    /// Allow the unsafe/deprecated --pdf-password command-line value
    #[arg(long = "allow-insecure-password-argv", action = clap::ArgAction::SetTrue)]
    pub(in crate::inbound::cli) allow_insecure_password_argv: bool,
    /// OCR language for a searchable PDF
    #[arg(long = "ocr-lang")]
    pub(in crate::inbound::cli) ocr_lang: Option<String>,
    /// OCR engine for a searchable PDF: offline, ocrs, or tesseract
    #[arg(long = "ocr-engine", value_enum)]
    pub(in crate::inbound::cli) ocr_engine: Option<OcrEngineArg>,
    /// Scanner profile JSON applied before export
    #[arg(long = "scanner-profile")]
    pub(in crate::inbound::cli) scanner_profile: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct Adjustments {
    #[arg(long)]
    pub(in crate::inbound::cli) rotate: Option<i32>,
    #[arg(long = "flip-h", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) flip_h: Option<bool>,
    #[arg(long = "flip-v", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) flip_v: Option<bool>,
    /// Crop as x,y,w,h in source pixels before rotate
    #[arg(long)]
    pub(in crate::inbound::cli) crop: Option<String>,
    #[arg(long)]
    pub(in crate::inbound::cli) brightness: Option<i32>,
    #[arg(long)]
    pub(in crate::inbound::cli) contrast: Option<i32>,
    /// Saturation delta from -100 to 100
    #[arg(long, value_parser = parse_saturation)]
    pub(in crate::inbound::cli) saturation: Option<f64>,
    /// Hue shift in degrees from -180 to 180
    #[arg(long, value_parser = parse_hue)]
    pub(in crate::inbound::cli) hue: Option<f64>,
    /// Tone curve as x:y,x:y (0..255 values; x values strictly increasing)
    #[arg(long, value_parser = parse_curves)]
    pub(in crate::inbound::cli) curves: Option<CurvePoints>,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) desaturate: Option<bool>,
    #[arg(long = "levels-black")]
    pub(in crate::inbound::cli) levels_black: Option<i32>,
    #[arg(long = "levels-white")]
    pub(in crate::inbound::cli) levels_white: Option<i32>,
    #[arg(long = "levels-gamma")]
    pub(in crate::inbound::cli) levels_gamma: Option<f64>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct FilterOptions {
    /// Infrared clean tier: off|light|medium|heavy
    #[arg(long = "infrared-clean")]
    pub(in crate::inbound::cli) infrared_clean: Option<String>,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) descreen: Option<bool>,
    #[arg(long = "descreen-dpi")]
    pub(in crate::inbound::cli) descreen_dpi: Option<i32>,
    #[arg(long)]
    pub(in crate::inbound::cli) sharpen: Option<f64>,
    #[arg(long = "film-type")]
    pub(in crate::inbound::cli) film_type: Option<String>,
    #[arg(long = "restore-colors", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) restore_colors: Option<bool>,
    #[arg(long = "restore-fading", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) restore_fading: Option<bool>,
    #[arg(long = "grain-reduction")]
    pub(in crate::inbound::cli) grain_reduction: Option<String>,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) flatten: Option<bool>,
    #[arg(long = "hole-punch", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) hole_punch: Option<bool>,
    #[arg(long = "colorize-mode")]
    pub(in crate::inbound::cli) colorize_mode: Option<String>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct AcquisitionOptions {
    #[arg(long)]
    pub(in crate::inbound::cli) device: Option<String>,
    /// Trust a strict direct eSCL endpoint that was not discovered or allow-listed
    #[arg(long = "allow-unlisted-escl", action = clap::ArgAction::SetTrue)]
    pub(in crate::inbound::cli) allow_unlisted_escl: bool,
    #[arg(long)]
    pub(in crate::inbound::cli) width: Option<u32>,
    #[arg(long)]
    pub(in crate::inbound::cli) height: Option<u32>,
    #[arg(long, default_value_t = 1)]
    pub(in crate::inbound::cli) seed: u32,
    #[arg(long)]
    pub(in crate::inbound::cli) dpi: Option<u32>,
    /// Hardware input source: flatbed, ADF, or film/transparency unit
    #[arg(long, value_enum)]
    pub(in crate::inbound::cli) source: Option<ScanSource>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct ScanOptions {
    #[command(flatten)]
    pub(in crate::inbound::cli) acquisition: AcquisitionOptions,
    #[arg(long, required = true)]
    pub(in crate::inbound::cli) out: PathBuf,
    #[command(flatten)]
    pub(in crate::inbound::cli) export: ExportOptions,
    /// Duplex returns multiple sides; use `batch --source adf --duplex`
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) duplex: Option<bool>,
    #[command(flatten)]
    pub(in crate::inbound::cli) adjustments: Adjustments,
    #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_deskew: Option<bool>,
    #[arg(long)]
    pub(in crate::inbound::cli) deskew: Option<f64>,
    #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) white_balance: Option<bool>,
    #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_levels: Option<bool>,
    #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_crop: Option<bool>,
    #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_orient: Option<bool>,
    #[command(flatten)]
    pub(in crate::inbound::cli) filters: FilterOptions,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) invert: Option<bool>,
    /// Save the pre-processing acquisition buffer (TIFF when extension is omitted)
    #[arg(long = "raw-out")]
    pub(in crate::inbound::cli) raw_out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct ProcessOptions {
    #[arg(long = "in")]
    pub(in crate::inbound::cli) inp: PathBuf,
    #[arg(long)]
    pub(in crate::inbound::cli) out: PathBuf,
    #[command(flatten)]
    pub(in crate::inbound::cli) export: ExportOptions,
    #[command(flatten)]
    pub(in crate::inbound::cli) adjustments: Adjustments,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) invert: Option<bool>,
    #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_levels: Option<bool>,
    #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_crop: Option<bool>,
    #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_orient: Option<bool>,
    #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_deskew: Option<bool>,
    #[arg(long)]
    pub(in crate::inbound::cli) deskew: Option<f64>,
    #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) white_balance: Option<bool>,
    #[command(flatten)]
    pub(in crate::inbound::cli) filters: FilterOptions,
    #[arg(long)]
    pub(in crate::inbound::cli) quality: Option<u8>,
}

#[derive(Debug, Args)]
pub(in crate::inbound::cli) struct BatchOptions {
    #[command(flatten)]
    pub(in crate::inbound::cli) acquisition: AcquisitionOptions,
    #[arg(long = "out-dir", required = true)]
    pub(in crate::inbound::cli) out_dir: PathBuf,
    /// Maximum logical image sides to acquire (1..=1000)
    #[arg(long)]
    pub(in crate::inbound::cli) pages: Option<u32>,
    /// Acquire both sides from an ADF; --pages must be even
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) duplex: Option<bool>,
    /// Per-page image format; defaults to the configured output format
    #[arg(long)]
    pub(in crate::inbound::cli) format: Option<String>,
    #[command(flatten)]
    pub(in crate::inbound::cli) adjustments: Adjustments,
    #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_deskew: Option<bool>,
    #[arg(long)]
    pub(in crate::inbound::cli) deskew: Option<f64>,
    #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_crop: Option<bool>,
    #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_orient: Option<bool>,
    #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) white_balance: Option<bool>,
    #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) auto_levels: Option<bool>,
    #[command(flatten)]
    pub(in crate::inbound::cli) filters: FilterOptions,
    #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
    pub(in crate::inbound::cli) invert: Option<bool>,
    #[arg(long = "multipage-tiff")]
    pub(in crate::inbound::cli) multipage_tiff: Option<PathBuf>,
    #[arg(long = "multipage-pdf")]
    pub(in crate::inbound::cli) multipage_pdf: Option<PathBuf>,
    /// Extension-driven multipage output (.pdf, .tif, or .tiff)
    #[arg(long = "multipage-out")]
    pub(in crate::inbound::cli) multipage_out: Option<PathBuf>,
    /// Add OCR text layers to the multipage PDF destination
    #[command(flatten)]
    pub(in crate::inbound::cli) export: ExportOptions,
    #[arg(long = "contact-sheet")]
    pub(in crate::inbound::cli) contact_sheet: Option<PathBuf>,
}
