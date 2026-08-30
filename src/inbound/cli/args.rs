use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(super) struct CurvePoints(pub(super) Vec<[i32; 2]>);

fn parse_saturation(value: &str) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "saturation must be a number from -100 to 100".to_string())?;
    crate::infrastructure::config::json::validate_saturation(value)
        .map_err(|error| error.to_string())
}

fn parse_hue(value: &str) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "hue must be a number of degrees from -180 to 180".to_string())?;
    crate::infrastructure::config::json::validate_hue(value).map_err(|error| error.to_string())
}

fn parse_curves(value: &str) -> Result<CurvePoints, String> {
    crate::infrastructure::config::json::parse_curve_points(value)
        .map_err(|error| error.to_string())?
        .map(CurvePoints)
        .ok_or_else(|| "curves must contain at least two x:y points".to_string())
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum ScanSource {
    Flatbed,
    Adf,
    Film,
}

impl ScanSource {
    pub(super) fn mode(self) -> crate::domain::acquisition::ScanMode {
        match self {
            Self::Flatbed => crate::domain::acquisition::ScanMode::Reflective,
            Self::Adf => crate::domain::acquisition::ScanMode::Document,
            Self::Film => crate::domain::acquisition::ScanMode::Film,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum OnnxLayout {
    Auto,
    Nchw,
    Nhwc,
}

impl OnnxLayout {
    pub(super) fn as_ml(self) -> crate::infrastructure::onnx::OnnxInputLayout {
        match self {
            Self::Auto => crate::infrastructure::onnx::OnnxInputLayout::Auto,
            Self::Nchw => crate::infrastructure::onnx::OnnxInputLayout::Nchw,
            Self::Nhwc => crate::infrastructure::onnx::OnnxInputLayout::Nhwc,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum OnnxNormalization {
    ZeroToOne,
    None,
}

impl OnnxNormalization {
    pub(super) fn as_ml(self) -> crate::infrastructure::onnx::OnnxNormalization {
        match self {
            Self::ZeroToOne => crate::infrastructure::onnx::OnnxNormalization::ZeroToOne,
            Self::None => crate::infrastructure::onnx::OnnxNormalization::None,
        }
    }
}

#[derive(Debug, Clone, ValueEnum)]
pub(super) enum RunMode {
    Normal,
    Plugin,
}

/// OCR implementation for a searchable PDF export.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum OcrEngineArg {
    Offline,
    Tesseract,
}

impl OcrEngineArg {
    pub(super) fn as_export(self) -> crate::OcrEngine {
        match self {
            Self::Offline => crate::OcrEngine::Offline,
            Self::Tesseract => crate::OcrEngine::Tesseract,
        }
    }
}

#[derive(Debug, Clone, ValueEnum)]
pub(super) enum InfoModule {
    All,
    Platform,
    Ocr,
    Ml,
    Twain,
    Backends,
    Features,
    Manufacturers,
}

#[derive(Debug, Parser)]
#[command(
    name = "open-scanline",
    about = "Local scanning, processing, and OCR in Rust.",
    version = env!("CARGO_PKG_VERSION")
)]
pub(super) struct Cli {
    /// Path to config.json (default: platform config dir)
    #[arg(long, global = true)]
    pub(super) config: Option<PathBuf>,

    /// Run mode: normal (default) or plugin (headless host entry)
    #[arg(long, global = true, value_enum, default_value_t = RunMode::Normal)]
    pub(super) mode: RunMode,

    #[command(subcommand)]
    pub(super) cmd: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub(super) enum Commands {
    /// Acquire an image from a device backend
    #[command(
        after_long_help = "Configuration-aware switches accept --flag or --flag=false. Omit a switch to retain its configured value."
    )]
    Scan {
        #[arg(long)]
        device: Option<String>,
        /// Trust a strict direct eSCL endpoint that was not discovered or allow-listed
        #[arg(long = "allow-unlisted-escl", action = clap::ArgAction::SetTrue)]
        allow_unlisted_escl: bool,
        #[arg(long, required = true)]
        out: PathBuf,
        /// Add an OCR text layer when exporting to PDF
        #[arg(long = "pdf-searchable", action = clap::ArgAction::SetTrue)]
        pdf_searchable: bool,
        /// UNSAFE/DEPRECATED: literal PDF password; requires explicit opt-in
        #[arg(
            long = "pdf-password",
            conflicts_with = "pdf_password_file",
            requires = "allow_insecure_password_argv"
        )]
        pdf_password: Option<String>,
        /// Read a PDF password from PATH, or - for standard input
        #[arg(long = "pdf-password-file", conflicts_with = "pdf_password")]
        pdf_password_file: Option<PathBuf>,
        /// Allow the unsafe/deprecated --pdf-password command-line value
        #[arg(long = "allow-insecure-password-argv", action = clap::ArgAction::SetTrue)]
        allow_insecure_password_argv: bool,
        /// OCR language for a searchable PDF
        #[arg(long = "ocr-lang")]
        ocr_lang: Option<String>,
        /// OCR engine for a searchable PDF: offline or tesseract
        #[arg(long = "ocr-engine", value_enum)]
        ocr_engine: Option<OcrEngineArg>,
        /// Scanner profile JSON applied before export
        #[arg(long = "scanner-profile")]
        scanner_profile: Option<PathBuf>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long, default_value_t = 1)]
        seed: u32,
        #[arg(long)]
        dpi: Option<u32>,
        /// Hardware input source: flatbed, ADF, or film/transparency unit
        #[arg(long, value_enum)]
        source: Option<ScanSource>,
        /// Duplex returns multiple sides; use `batch --source adf --duplex`
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        duplex: Option<bool>,
        #[arg(long)]
        rotate: Option<i32>,
        #[arg(long = "flip-h", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_h: Option<bool>,
        #[arg(long = "flip-v", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_v: Option<bool>,
        /// Crop as x,y,w,h in source pixels before rotate
        #[arg(long)]
        crop: Option<String>,
        #[arg(long)]
        brightness: Option<i32>,
        #[arg(long)]
        contrast: Option<i32>,
        /// Saturation delta from -100 to 100
        #[arg(long, value_parser = parse_saturation)]
        saturation: Option<f64>,
        /// Hue shift in degrees from -180 to 180
        #[arg(long, value_parser = parse_hue)]
        hue: Option<f64>,
        /// Tone curve as x:y,x:y (0..255 values; x values strictly increasing)
        #[arg(long, value_parser = parse_curves)]
        curves: Option<CurvePoints>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        desaturate: Option<bool>,
        #[arg(long = "levels-black")]
        levels_black: Option<i32>,
        #[arg(long = "levels-white")]
        levels_white: Option<i32>,
        #[arg(long = "levels-gamma")]
        levels_gamma: Option<f64>,
        #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_deskew: Option<bool>,
        #[arg(long)]
        deskew: Option<f64>,
        #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        white_balance: Option<bool>,
        #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_levels: Option<bool>,
        #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_crop: Option<bool>,
        #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_orient: Option<bool>,
        /// Infrared clean tier: off|light|medium|heavy
        #[arg(long = "infrared-clean")]
        infrared_clean: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        descreen: Option<bool>,
        #[arg(long = "descreen-dpi")]
        descreen_dpi: Option<i32>,
        #[arg(long)]
        sharpen: Option<f64>,
        #[arg(long = "film-type")]
        film_type: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        invert: Option<bool>,
        #[arg(long = "restore-colors", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_colors: Option<bool>,
        #[arg(long = "restore-fading", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_fading: Option<bool>,
        #[arg(long = "grain-reduction")]
        grain_reduction: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flatten: Option<bool>,
        #[arg(long = "hole-punch", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        hole_punch: Option<bool>,
        #[arg(long = "colorize-mode")]
        colorize_mode: Option<String>,
        /// Save the pre-processing acquisition buffer (TIFF when extension is omitted)
        #[arg(long = "raw-out")]
        raw_out: Option<PathBuf>,
    },
    /// List available devices
    Devices,
    /// Open manufacturer support catalog
    Manufacturers {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        resolve: Option<String>,
    },
    /// Show or write config
    Config {
        #[arg(long)]
        init: bool,
        #[arg(long)]
        show: bool,
        #[arg(long = "set-output-dir")]
        set_output_dir: Option<String>,
        #[arg(long = "set-dpi")]
        set_dpi: Option<u32>,
        #[arg(long = "set-device")]
        set_device: Option<String>,
    },
    /// Launch desktop GUI
    Gui,
    /// Plugin/host mode (headless; same as --mode=plugin)
    Plugin {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        device: Option<String>,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        quiet: bool,
    },
    /// Convert image formats via real codec path
    Convert {
        #[arg(long = "in")]
        inp: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        dpi: Option<u32>,
    },
    /// OCR an image file
    Ocr {
        #[arg(long = "in")]
        inp: PathBuf,
        #[arg(long, default_value = "eng")]
        lang: String,
        #[arg(long, action = clap::ArgAction::SetTrue)]
        offline: bool,
    },
    /// Run a user-supplied ONNX image model and print its inference report
    Onnx {
        #[arg(long = "in")]
        inp: PathBuf,
        #[arg(long)]
        model: PathBuf,
        /// Named model input; required only for multi-input models
        #[arg(long = "input-name")]
        input_name: Option<String>,
        #[arg(long, value_enum, default_value_t = OnnxLayout::Auto)]
        layout: OnnxLayout,
        #[arg(long, value_enum, default_value_t = OnnxNormalization::ZeroToOne)]
        normalization: OnnxNormalization,
    },
    /// Internal resource-contained ONNX worker
    #[command(name = "__onnx-worker", hide = true)]
    OnnxWorker {
        #[arg(long = "in", hide = true)]
        inp: PathBuf,
        #[arg(long, hide = true)]
        model: PathBuf,
        #[arg(long = "report", hide = true)]
        report: PathBuf,
        #[arg(long = "input-name", hide = true)]
        input_name: Option<String>,
        #[arg(long, value_enum, default_value_t = OnnxLayout::Auto, hide = true)]
        layout: OnnxLayout,
        #[arg(long, value_enum, default_value_t = OnnxNormalization::ZeroToOne, hide = true)]
        normalization: OnnxNormalization,
        #[arg(long = "worker-protocol", hide = true)]
        worker_protocol: String,
    },
    /// Pipeline process an image file
    #[command(
        after_long_help = "Configuration-aware switches accept --flag or --flag=false. Omit a switch to retain its configured value. Use off or none for optional processing tiers, film type, and colorization mode."
    )]
    Process {
        #[arg(long = "in")]
        inp: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Add an OCR text layer when exporting to PDF
        #[arg(long = "pdf-searchable", action = clap::ArgAction::SetTrue)]
        pdf_searchable: bool,
        /// UNSAFE/DEPRECATED: literal PDF password; requires explicit opt-in
        #[arg(
            long = "pdf-password",
            conflicts_with = "pdf_password_file",
            requires = "allow_insecure_password_argv"
        )]
        pdf_password: Option<String>,
        /// Read a PDF password from PATH, or - for standard input
        #[arg(long = "pdf-password-file", conflicts_with = "pdf_password")]
        pdf_password_file: Option<PathBuf>,
        /// Allow the unsafe/deprecated --pdf-password command-line value
        #[arg(long = "allow-insecure-password-argv", action = clap::ArgAction::SetTrue)]
        allow_insecure_password_argv: bool,
        /// OCR language for a searchable PDF
        #[arg(long = "ocr-lang")]
        ocr_lang: Option<String>,
        /// OCR engine for a searchable PDF: offline or tesseract
        #[arg(long = "ocr-engine", value_enum)]
        ocr_engine: Option<OcrEngineArg>,
        /// Scanner profile JSON applied before export
        #[arg(long = "scanner-profile")]
        scanner_profile: Option<PathBuf>,
        #[arg(long)]
        rotate: Option<i32>,
        #[arg(long = "flip-h", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_h: Option<bool>,
        #[arg(long = "flip-v", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_v: Option<bool>,
        #[arg(long)]
        crop: Option<String>,
        #[arg(long)]
        brightness: Option<i32>,
        #[arg(long)]
        contrast: Option<i32>,
        /// Saturation delta from -100 to 100
        #[arg(long, value_parser = parse_saturation)]
        saturation: Option<f64>,
        /// Hue shift in degrees from -180 to 180
        #[arg(long, value_parser = parse_hue)]
        hue: Option<f64>,
        /// Tone curve as x:y,x:y (0..255 values; x values strictly increasing)
        #[arg(long, value_parser = parse_curves)]
        curves: Option<CurvePoints>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        desaturate: Option<bool>,
        #[arg(long = "levels-black")]
        levels_black: Option<i32>,
        #[arg(long = "levels-white")]
        levels_white: Option<i32>,
        #[arg(long = "levels-gamma")]
        levels_gamma: Option<f64>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        invert: Option<bool>,
        #[arg(long)]
        sharpen: Option<f64>,
        #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_levels: Option<bool>,
        #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_crop: Option<bool>,
        #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_orient: Option<bool>,
        #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_deskew: Option<bool>,
        #[arg(long)]
        deskew: Option<f64>,
        #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        white_balance: Option<bool>,
        /// Infrared clean tier: off|light|medium|heavy
        #[arg(long = "infrared-clean")]
        infrared_clean: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        descreen: Option<bool>,
        #[arg(long = "descreen-dpi")]
        descreen_dpi: Option<i32>,
        #[arg(long = "film-type")]
        film_type: Option<String>,
        #[arg(long = "restore-colors", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_colors: Option<bool>,
        #[arg(long = "restore-fading", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_fading: Option<bool>,
        #[arg(long = "grain-reduction")]
        grain_reduction: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flatten: Option<bool>,
        #[arg(long = "hole-punch", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        hole_punch: Option<bool>,
        #[arg(long = "colorize-mode")]
        colorize_mode: Option<String>,
        #[arg(long)]
        quality: Option<u8>,
    },
    /// Multi-page batch scan via shared path
    #[command(
        after_long_help = "Configuration-aware switches accept --flag or --flag=false. Omit a switch to retain its configured value. Use off or none for optional processing tiers, film type, and colorization mode."
    )]
    Batch {
        /// Device id; defaults to the configured last device
        #[arg(long)]
        device: Option<String>,
        /// Trust a strict direct eSCL endpoint that was not discovered or allow-listed
        #[arg(long = "allow-unlisted-escl", action = clap::ArgAction::SetTrue)]
        allow_unlisted_escl: bool,
        #[arg(long = "out-dir", required = true)]
        out_dir: PathBuf,
        /// Maximum logical image sides to acquire (1..=1000)
        #[arg(long)]
        pages: Option<u32>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long, default_value_t = 1)]
        seed: u32,
        #[arg(long)]
        dpi: Option<u32>,
        /// Hardware input source: flatbed, ADF, or film/transparency unit
        #[arg(long, value_enum)]
        source: Option<ScanSource>,
        /// Acquire both sides from an ADF; --pages must be even
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        duplex: Option<bool>,
        /// Per-page image format; defaults to the configured output format
        #[arg(long)]
        format: Option<String>,
        #[arg(long)]
        rotate: Option<i32>,
        #[arg(long = "flip-h", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_h: Option<bool>,
        #[arg(long = "flip-v", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flip_v: Option<bool>,
        /// Crop as x,y,w,h in source pixels before rotate
        #[arg(long)]
        crop: Option<String>,
        #[arg(long)]
        brightness: Option<i32>,
        #[arg(long)]
        contrast: Option<i32>,
        #[arg(long, value_parser = parse_saturation)]
        saturation: Option<f64>,
        #[arg(long, value_parser = parse_hue)]
        hue: Option<f64>,
        #[arg(long, value_parser = parse_curves)]
        curves: Option<CurvePoints>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        desaturate: Option<bool>,
        #[arg(long = "levels-black")]
        levels_black: Option<i32>,
        #[arg(long = "levels-white")]
        levels_white: Option<i32>,
        #[arg(long = "levels-gamma")]
        levels_gamma: Option<f64>,
        #[arg(long = "auto-deskew", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_deskew: Option<bool>,
        #[arg(long)]
        deskew: Option<f64>,
        #[arg(long = "auto-crop", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_crop: Option<bool>,
        #[arg(long = "auto-orient", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_orient: Option<bool>,
        #[arg(long = "white-balance", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        white_balance: Option<bool>,
        #[arg(long = "auto-levels", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        auto_levels: Option<bool>,
        #[arg(long = "infrared-clean")]
        infrared_clean: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        descreen: Option<bool>,
        #[arg(long = "descreen-dpi")]
        descreen_dpi: Option<i32>,
        #[arg(long)]
        sharpen: Option<f64>,
        #[arg(long = "film-type")]
        film_type: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        invert: Option<bool>,
        #[arg(long = "restore-colors", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_colors: Option<bool>,
        #[arg(long = "restore-fading", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        restore_fading: Option<bool>,
        #[arg(long = "grain-reduction")]
        grain_reduction: Option<String>,
        #[arg(long, action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        flatten: Option<bool>,
        #[arg(long = "hole-punch", action = clap::ArgAction::Set, default_missing_value = "true", num_args = 0..=1, require_equals = true)]
        hole_punch: Option<bool>,
        #[arg(long = "colorize-mode")]
        colorize_mode: Option<String>,
        #[arg(long = "multipage-tiff")]
        multipage_tiff: Option<PathBuf>,
        #[arg(long = "multipage-pdf")]
        multipage_pdf: Option<PathBuf>,
        /// Extension-driven multipage output (.pdf, .tif, or .tiff)
        #[arg(long = "multipage-out")]
        multipage_out: Option<PathBuf>,
        /// Add OCR text layers to the multipage PDF destination
        #[arg(long = "pdf-searchable", action = clap::ArgAction::SetTrue)]
        pdf_searchable: bool,
        /// UNSAFE/DEPRECATED: literal PDF password; requires explicit opt-in
        #[arg(
            long = "pdf-password",
            conflicts_with = "pdf_password_file",
            requires = "allow_insecure_password_argv"
        )]
        pdf_password: Option<String>,
        /// Read a PDF password from PATH, or - for standard input
        #[arg(long = "pdf-password-file", conflicts_with = "pdf_password")]
        pdf_password_file: Option<PathBuf>,
        /// Allow the unsafe/deprecated --pdf-password command-line value
        #[arg(long = "allow-insecure-password-argv", action = clap::ArgAction::SetTrue)]
        allow_insecure_password_argv: bool,
        /// OCR language for a searchable multipage PDF
        #[arg(long = "ocr-lang")]
        ocr_lang: Option<String>,
        /// OCR engine for a searchable multipage PDF: offline or tesseract
        #[arg(long = "ocr-engine", value_enum)]
        ocr_engine: Option<OcrEngineArg>,
        /// Scanner profile JSON applied to every exported page
        #[arg(long = "scanner-profile")]
        scanner_profile: Option<PathBuf>,
        #[arg(long = "contact-sheet")]
        contact_sheet: Option<PathBuf>,
    },
    /// Create and validate a portable ZIP around an existing executable
    Package {
        #[arg(long, required = true)]
        binary: PathBuf,
        #[arg(long, required = true)]
        out: PathBuf,
    },
    /// Print module/platform capability JSON
    Info {
        #[arg(long, value_enum, default_value_t = InfoModule::All)]
        module: InfoModule,
    },
    /// Print built-in user manual text
    #[command(name = "help-text")]
    HelpText,
}
