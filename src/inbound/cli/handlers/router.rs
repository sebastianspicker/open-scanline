mod export;
mod routes;

use super::{cancellation, commands, info};
use crate::inbound::cli::args::{Commands, OcrModelCommand};
use std::path::{Path, PathBuf};

pub(super) fn dispatch_command(command: Commands, config_path: Option<&Path>) -> i32 {
    match command {
        Commands::Scan { options } => routes::scan(options, config_path),
        Commands::Process { options } => routes::process(options, config_path),
        Commands::Batch { options } => routes::batch(options, config_path),
        command => dispatch_non_pipeline(command, config_path),
    }
}

fn dispatch_non_pipeline(command: Commands, config_path: Option<&Path>) -> i32 {
    if is_tooling_command(&command) {
        return dispatch_tooling(command);
    }
    dispatch_local(command, config_path)
}

fn is_tooling_command(command: &Commands) -> bool {
    matches!(
        command,
        Commands::OcrModel { .. }
            | Commands::Onnx { .. }
            | Commands::OnnxWorker { .. }
            | Commands::Package { .. }
            | Commands::HelpText
            | Commands::Info { .. }
    )
}

fn dispatch_local(command: Commands, config_path: Option<&Path>) -> i32 {
    match command {
        Commands::Devices => commands::devices(),
        Commands::Manufacturers { json, resolve } => commands::manufacturers(json, resolve),
        Commands::Config {
            init,
            show,
            set_output_dir,
            set_dpi,
            set_device,
        } => commands::config(commands::ConfigRequest {
            config_path,
            init,
            show,
            set_output_dir,
            set_dpi,
            set_device,
        }),
        Commands::Gui => commands::gui(config_path),
        Commands::Plugin { out, device, quiet } => plugin(config_path, out, device, quiet),
        Commands::Convert { inp, out, dpi } => convert(inp, out, dpi),
        Commands::Ocr {
            inp,
            lang,
            offline,
            engine,
        } => ocr(inp, lang, offline, engine),
        command => unreachable!("tooling command reached local dispatcher: {command:?}"),
    }
}

fn dispatch_tooling(command: Commands) -> i32 {
    match command {
        Commands::OcrModel { command } => ocr_model(command),
        Commands::Onnx {
            inp,
            model,
            input_name,
            layout,
            normalization,
        } => commands::onnx(inp, model, input_name, layout, normalization),
        Commands::OnnxWorker {
            inp,
            model,
            report,
            input_name,
            layout,
            normalization,
            worker_protocol,
        } => commands::onnx_worker(
            inp,
            model,
            report,
            input_name,
            layout,
            normalization,
            worker_protocol,
        ),
        Commands::Package { binary, out } => commands::package(binary, out),
        Commands::HelpText => commands::help_text(),
        Commands::Info { module } => info::run(module),
        command => unreachable!("local command reached tooling dispatcher: {command:?}"),
    }
}

fn plugin(
    config_path: Option<&Path>,
    out: Option<PathBuf>,
    device: Option<String>,
    quiet: bool,
) -> i32 {
    cancellation::with_registered_token(move |cancellation| {
        commands::plugin(config_path, out, device, quiet, cancellation)
    })
}

fn convert(inp: PathBuf, out: PathBuf, dpi: Option<u32>) -> i32 {
    cancellation::with_registered_token(move |cancellation| {
        commands::convert(inp, out, dpi, cancellation)
    })
}

fn ocr(
    inp: PathBuf,
    lang: String,
    offline: bool,
    engine: crate::inbound::cli::args::OcrEngineArg,
) -> i32 {
    cancellation::with_registered_token(move |cancellation| {
        commands::ocr(
            inp,
            lang,
            if offline {
                crate::OcrEngine::Offline
            } else {
                engine.as_export()
            },
            cancellation,
        )
    })
}

fn ocr_model(command: OcrModelCommand) -> i32 {
    match command {
        OcrModelCommand::Install {
            detection,
            recognition,
        } => commands::ocr_model_install(detection, recognition),
        OcrModelCommand::Status => commands::ocr_model_status(),
    }
}

pub(super) fn clear_pdf_password(pdf_password: &mut Option<String>) {
    export::clear_pdf_password(pdf_password);
}
