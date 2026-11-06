mod options;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

pub(super) use options::{
    Adjustments, BatchOptions, ExportOptions, FilterOptions, ProcessOptions, ScanOptions,
};

#[derive(Debug, Clone)]
pub(super) struct CurvePoints(pub(super) Vec<[i32; 2]>);

fn parse_saturation(value: &str) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "saturation must be a number from -100 to 100".to_string())?;
    crate::domain::settings::validate_saturation(value).map_err(|error| error.to_string())
}

fn parse_hue(value: &str) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "hue must be a number of degrees from -180 to 180".to_string())?;
    crate::domain::settings::validate_hue(value).map_err(|error| error.to_string())
}

fn parse_rotate(value: &str) -> Result<i32, String> {
    let degrees = value
        .parse::<i32>()
        .map_err(|_| "rotate must be a whole number of degrees".to_string())?;
    if degrees % 90 != 0 {
        return Err("rotate must be a multiple of 90 degrees".into());
    }
    Ok(degrees)
}

fn parse_levels_gamma(value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(gamma) if gamma.is_finite() && gamma > 0.0 => Ok(gamma),
        _ => Err("levels gamma must be a finite number greater than 0".into()),
    }
}

fn parse_deskew(value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(angle) if angle.is_finite() && angle.abs() <= 45.0 => Ok(angle),
        _ => Err("deskew must be a number of degrees from -45 to 45".into()),
    }
}

fn parse_int(value: &str, label: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .map_err(|_| format!("{label} must be a whole number"))
}

fn parse_brightness(value: &str) -> Result<i32, String> {
    crate::domain::processing::validate_adjustment("brightness", parse_int(value, "brightness")?)
        .map_err(|error| error.to_string())
}

fn parse_contrast(value: &str) -> Result<i32, String> {
    crate::domain::processing::validate_adjustment("contrast", parse_int(value, "contrast")?)
        .map_err(|error| error.to_string())
}

/// Levels points are checked 0..=255 here; black < white is checked once the
/// configured value is known (`validate_levels`).
fn parse_levels_point(value: &str) -> Result<i32, String> {
    match parse_int(value, "levels value")? {
        point @ 0..=255 => Ok(point),
        _ => Err("levels black and white must be from 0 to 255".into()),
    }
}

fn parse_sharpen(value: &str) -> Result<f64, String> {
    let amount = value
        .parse::<f64>()
        .map_err(|_| "sharpen must be a number from 0 to 5".to_string())?;
    crate::domain::processing::validate_sharpen(amount).map_err(|error| error.to_string())
}

fn parse_descreen_dpi(value: &str) -> Result<i32, String> {
    crate::domain::processing::validate_descreen_dpi(parse_int(value, "descreen dpi")?)
        .map_err(|error| error.to_string())
}

fn parse_choice(value: &str, label: &str, allowed: &[&str]) -> Result<String, String> {
    crate::domain::processing::validate_disableable_choice(label, value, allowed)
        .map_err(|error| error.to_string())?;
    Ok(value.to_string())
}

fn parse_filter_tier(value: &str) -> Result<String, String> {
    crate::domain::processing::validate_filter_tier("tier", value)
        .map_err(|error| error.to_string())?;
    Ok(value.to_string())
}

fn parse_colorize_mode(value: &str) -> Result<String, String> {
    parse_choice(
        value,
        "colorize mode",
        crate::domain::processing::COLORIZE_MODES,
    )
}

fn parse_crop(value: &str) -> Result<[i32; 4], String> {
    let crop = crate::domain::image::parse_crop_components(value).map_err(|error| match error {
        crate::domain::image::CropParseError::ComponentCount => "crop must be x,y,w,h".to_string(),
        crate::domain::image::CropParseError::InvalidInteger(_) => {
            "crop must be x,y,w,h integers".to_string()
        }
    })?;
    // Negative extents cannot become a `u32` size; reject them before the domain check.
    let (Ok(width), Ok(height)) = (u32::try_from(crop[2]), u32::try_from(crop[3])) else {
        return Err("invalid crop region".into());
    };
    let region = crate::domain::image::Rect::new(crop[0], crop[1], width, height);
    crate::domain::processing::validate_crop_region(region).map_err(|error| error.to_string())?;
    Ok(crop)
}

fn parse_paper(value: &str) -> Result<crate::domain::acquisition::PaperSize, String> {
    crate::domain::acquisition::PaperSize::parse(value).map_err(|error| error.to_string())
}

fn parse_curves(value: &str) -> Result<CurvePoints, String> {
    crate::domain::settings::parse_curve_points(value)
        .map_err(|error| error.to_string())?
        .map(CurvePoints)
        .ok_or_else(|| "curves must contain at least two x:y points".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(super) enum OcrEngineArg {
    /// Built-in 5x7 uppercase template matcher; not suitable for real documents
    Offline,
    /// Local OCRS model pack (printed Latin English)
    Ocrs,
    /// System Tesseract executable
    Tesseract,
}

impl OcrEngineArg {
    pub(super) fn as_export(self) -> crate::OcrEngine {
        match self {
            Self::Offline => crate::OcrEngine::Offline,
            Self::Ocrs => crate::OcrEngine::Ocrs,
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

#[derive(Debug, Subcommand)]
pub(super) enum OcrModelCommand {
    /// Validate and install a local OCRS RTen detection/recognition pair.
    Install {
        #[arg(long)]
        detection: PathBuf,
        #[arg(long)]
        recognition: PathBuf,
    },
    /// Report the active OCRS model pack and verify its integrity.
    Status,
}

#[derive(Debug, Parser)]
#[command(name = "open-scanline", about = "Local scanning, processing, and OCR in Rust.", version = env!("CARGO_PKG_VERSION"))]
pub(super) struct Cli {
    /// Path to config.json (default: platform config dir)
    #[arg(long, global = true)]
    pub(super) config: Option<PathBuf>,
    /// Run mode: normal (default) or plugin (headless host entry)
    #[arg(long, global = true, value_enum, default_value_t = RunMode::Normal)]
    pub(super) mode: RunMode,
    /// Print backend diagnostics to stderr (also OPEN_SCANLINE_DEBUG=1)
    #[arg(long, global = true, action = clap::ArgAction::SetTrue)]
    pub(super) verbose: bool,
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
        #[command(flatten)]
        options: ScanOptions,
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
        #[arg(long = "set-paper")]
        set_paper: Option<String>,
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
        /// Replace an existing output file instead of failing
        #[arg(long, action = clap::ArgAction::SetTrue)]
        overwrite: bool,
    },
    /// OCR an image file
    Ocr {
        #[arg(long = "in")]
        inp: PathBuf,
        #[arg(long, default_value = "eng")]
        lang: String,
        /// Deprecated alias for `--engine offline`.
        #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "engine")]
        offline: bool,
        /// OCR engine: offline, ocrs, or tesseract.
        #[arg(long, value_enum, default_value_t = OcrEngineArg::Tesseract, conflicts_with = "offline")]
        engine: OcrEngineArg,
    },
    /// Manage locally installed OCRS model packs. This command never downloads models.
    #[command(name = "ocr-model")]
    OcrModel {
        #[command(subcommand)]
        command: OcrModelCommand,
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
        #[command(flatten)]
        options: ProcessOptions,
    },
    /// Multi-page batch scan via shared path
    #[command(
        after_long_help = "Configuration-aware switches accept --flag or --flag=false. Omit a switch to retain its configured value. Use off or none for optional processing tiers, film type, and colorization mode."
    )]
    Batch {
        #[command(flatten)]
        options: BatchOptions,
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

#[cfg(test)]
mod ocr_contract_tests {
    use super::*;

    #[test]
    fn direct_ocr_defaults_to_tesseract_and_accepts_each_explicit_engine() {
        let cli = Cli::try_parse_from(["open-scanline", "ocr", "--in", "page.png"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Commands::Ocr {
                offline: false,
                engine: OcrEngineArg::Tesseract,
                ..
            })
        ));
        for (value, expected) in [
            ("offline", OcrEngineArg::Offline),
            ("ocrs", OcrEngineArg::Ocrs),
            ("tesseract", OcrEngineArg::Tesseract),
        ] {
            let cli = Cli::try_parse_from([
                "open-scanline",
                "ocr",
                "--in",
                "page.png",
                "--engine",
                value,
            ])
            .unwrap();
            assert!(
                matches!(cli.cmd, Some(Commands::Ocr { offline: false, engine, .. }) if engine == expected)
            );
        }
    }

    #[test]
    fn deprecated_offline_alias_does_not_conflict_with_engine_default_but_rejects_an_explicit_engine(
    ) {
        let cli =
            Cli::try_parse_from(["open-scanline", "ocr", "--in", "page.png", "--offline"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Commands::Ocr {
                offline: true,
                engine: OcrEngineArg::Tesseract,
                ..
            })
        ));
        assert!(Cli::try_parse_from([
            "open-scanline",
            "ocr",
            "--in",
            "page.png",
            "--offline",
            "--engine",
            "ocrs"
        ])
        .is_err());
    }

    #[test]
    fn ocr_model_status_parses_without_paths() {
        let cli = Cli::try_parse_from(["open-scanline", "ocr-model", "status"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Commands::OcrModel {
                command: OcrModelCommand::Status
            })
        ));
    }

    #[test]
    fn scan_invert_accepts_explicit_true_and_false_overrides() {
        for (value, expected) in [("true", true), ("false", false)] {
            let cli = Cli::try_parse_from([
                "open-scanline",
                "scan",
                "--out",
                "page.png",
                &format!("--invert={value}"),
            ])
            .unwrap();
            assert!(
                matches!(cli.cmd, Some(Commands::Scan { options }) if options.invert == Some(expected))
            );
        }
    }
}

#[cfg(test)]
mod crop_and_overwrite_tests {
    use super::*;

    #[test]
    fn rotate_gamma_and_deskew_are_validated_by_the_parser() {
        let parse = |flag: &str, value: &str| {
            Cli::try_parse_from([
                "open-scanline",
                "process",
                "--in",
                "a.png",
                "--out",
                "b.png",
                flag,
                value,
            ])
        };
        for (flag, good) in [
            ("--rotate", "-270"),
            ("--levels-gamma", "1.5"),
            ("--deskew", "-45"),
        ] {
            assert!(parse(flag, good).is_ok(), "{flag} {good}");
        }
        for (flag, bad) in [
            ("--rotate", "45"),
            ("--levels-gamma", "nan"),
            ("--levels-gamma", "0"),
            ("--levels-gamma", "-1"),
            ("--deskew", "inf"),
            ("--deskew", "46"),
        ] {
            assert!(parse(flag, bad).is_err(), "{flag} {bad} must be rejected");
        }
    }

    #[test]
    fn crop_is_validated_by_the_parser() {
        let cli = Cli::try_parse_from([
            "open-scanline",
            "scan",
            "--out",
            "a.png",
            "--crop",
            "1,2,3,4",
        ])
        .unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Commands::Scan { options }) if options.adjustments.crop == Some([1, 2, 3, 4])
        ));
        for bad in ["1,2,0,4", "1,2,3,0", "1,2,3", "a,b,c,d"] {
            assert!(
                Cli::try_parse_from(["open-scanline", "scan", "--out", "a.png", "--crop", bad])
                    .is_err(),
                "{bad} must be rejected"
            );
        }
    }

    #[test]
    fn overwrite_defaults_to_false_and_is_opt_in() {
        let cli = Cli::try_parse_from(["open-scanline", "scan", "--out", "a.png"]).unwrap();
        assert!(matches!(cli.cmd, Some(Commands::Scan { options }) if !options.overwrite));
        let cli = Cli::try_parse_from([
            "open-scanline",
            "convert",
            "--in",
            "a",
            "--out",
            "b",
            "--overwrite",
        ])
        .unwrap();
        assert!(matches!(
            cli.cmd,
            Some(Commands::Convert {
                overwrite: true,
                ..
            })
        ));
    }

    #[test]
    fn out_of_range_numbers_and_unknown_modes_are_rejected_by_the_parser() {
        let parse = |flag: &str, value: &str| {
            Cli::try_parse_from([
                "open-scanline",
                "process",
                "--in",
                "a.png",
                "--out",
                "b.png",
                flag,
                value,
            ])
        };
        for (flag, good) in [
            ("--brightness", "-100"),
            ("--contrast", "100"),
            ("--sharpen", "5"),
            ("--descreen-dpi", "300"),
            ("--levels-white", "255"),
            ("--grain-reduction", "off"),
            ("--infrared-clean", "Heavy"),
            ("--colorize-mode", "blue"),
        ] {
            assert!(parse(flag, good).is_ok(), "{flag} {good}");
        }
        for (flag, bad) in [
            ("--brightness", "500"),
            ("--contrast", "-101"),
            ("--sharpen", "-1"),
            ("--sharpen", "6"),
            ("--descreen-dpi", "10"),
            ("--levels-black", "256"),
            ("--grain-reduction", "hevy"),
            ("--infrared-clean", "heavyy"),
            ("--colorize-mode", "red"),
            ("--crop", "-1,0,5,5"),
            ("--crop", "0,-1,5,5"),
        ] {
            assert!(parse(flag, bad).is_err(), "{flag} {bad} must be rejected");
        }
    }

    #[test]
    fn pdf_password_conflicts_and_requirements_are_enforced_by_clap() {
        let parse = |extra: &[&str]| {
            let mut args = vec!["open-scanline", "process", "--in", "a", "--out", "b.pdf"];
            args.extend_from_slice(extra);
            Cli::try_parse_from(args)
        };
        assert!(parse(&["--pdf-password", "x"]).is_err());
        assert!(parse(&["--pdf-password", "x", "--allow-insecure-password-argv"]).is_ok());
        assert!(parse(&[
            "--pdf-password",
            "x",
            "--allow-insecure-password-argv",
            "--pdf-password-file",
            "p"
        ])
        .is_err());
    }
}
