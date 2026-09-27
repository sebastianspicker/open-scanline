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
    Offline,
    Ocrs,
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
