use super::super::manual::user_manual_text;
use super::common::print_wrote;
use crate::batch::{run_batch_scan_with_export_options_and_token_and_policy, BatchScanArgs};
use crate::cli::args::{OnnxLayout, OnnxNormalization, ScanSource};
use crate::cli::handlers::scan;
use crate::config::{default_config_path, load_config, save_config, AppConfig};
use crate::device::{list_all_devices, list_backends};
use crate::device::{CancellationToken, DeviceOpenPolicy};
use crate::gui::run_gui;
use crate::imaging::convert_image_with_cancellation;
use crate::manufacturers::{
    format_manufacturers_text, list_manufacturers, manufacturer_support_summary,
    resolve_manufacturer,
};
use crate::ml::{run_isolated_onnx_with_executable, run_onnx_worker, OnnxInferenceOptions};
use crate::ocr::ocr_file_with_cancellation;
use crate::packaging::{build_portable, PackagingOptions};
use crate::plugin::run_plugin_mode_with_token;
use std::path::{Path, PathBuf};

pub(super) fn devices() -> i32 {
    for b in list_backends() {
        let avail = if b.available { "true" } else { "false" };
        println!("# backend\t{}\t{}\tavailable={avail}", b.id, b.name);
    }
    for d in list_all_devices() {
        let mfr = d.manufacturer_id.as_deref().unwrap_or("-");
        println!("{}\t{}\t{}\tmanufacturer={mfr}", d.id, d.name, d.kind);
    }
    0
}

pub(super) fn manufacturers(json: bool, resolve: Option<String>) -> i32 {
    if let Some(q) = resolve {
        let result = resolve_manufacturer(&q);
        println!(
            "{}",
            serde_json::to_string_pretty(&result.as_dict()).unwrap_or_else(|_| "{}".into())
        );
        return if result.status == "ok" { 0 } else { 1 };
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&manufacturer_support_summary())
                .unwrap_or_else(|_| "{}".into())
        );
    } else {
        print!("{}", format_manufacturers_text(&list_manufacturers()));
    }
    0
}

pub(super) struct ConfigRequest<'a> {
    pub(super) config_path: Option<&'a Path>,
    pub(super) init: bool,
    pub(super) show: bool,
    pub(super) set_output_dir: Option<String>,
    pub(super) set_dpi: Option<u32>,
    pub(super) set_device: Option<String>,
}

pub(super) fn config(request: ConfigRequest<'_>) -> i32 {
    let path = request
        .config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);
    if request.should_write() {
        return write_config(&path, request);
    }
    show_config(&path, request.show);
    0
}

impl ConfigRequest<'_> {
    fn should_write(&self) -> bool {
        self.init
            || self.set_output_dir.is_some()
            || self.set_dpi.is_some()
            || self.set_device.is_some()
    }
}

fn write_config(path: &Path, request: ConfigRequest<'_>) -> i32 {
    let mut config = match load_config(Some(path)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("config error: {error}");
            return 1;
        }
    };
    apply_config_updates(&mut config, request);
    match save_config(&config, path) {
        Ok(saved) => {
            println!("wrote config {}", saved.display());
            0
        }
        Err(error) => {
            eprintln!("config error: {error}");
            1
        }
    }
}

fn apply_config_updates(config: &mut AppConfig, request: ConfigRequest<'_>) {
    if let Some(output_dir) = request.set_output_dir {
        config.output_dir = output_dir;
    }
    if let Some(dpi) = request.set_dpi {
        config.default_dpi = dpi;
    }
    if let Some(device) = request.set_device {
        config.last_device_id = device;
    }
}

fn show_config(path: &Path, show: bool) {
    println!("config_path={}", path.display());
    if path.is_file() {
        match std::fs::read_to_string(path) {
            Ok(text) => print!("{text}"),
            Err(error) => eprintln!("{error}"),
        }
    } else {
        println!("(file not found; defaults in use)");
        let config = load_config(Some(path)).unwrap_or_default();
        println!("{config:?}");
    }
    let _ = show;
}

pub(super) fn gui(config_path: Option<&Path>) -> i32 {
    run_gui(config_path)
}

pub(super) fn plugin(
    config_path: Option<&Path>,
    out: Option<PathBuf>,
    device: Option<String>,
    quiet: bool,
    cancellation: CancellationToken,
) -> i32 {
    run_plugin_mode_with_token(
        config_path,
        out.as_deref(),
        quiet,
        device.as_deref(),
        cancellation,
    )
}

pub(super) fn convert(
    inp: PathBuf,
    out: PathBuf,
    dpi: Option<u32>,
    cancellation: CancellationToken,
) -> i32 {
    match convert_image_with_cancellation(&inp, &out, dpi, cancellation) {
        Ok(path) => {
            print_wrote(&path);
            0
        }
        Err(e) => {
            eprintln!("convert error: {e}");
            1
        }
    }
}

pub(super) fn ocr(
    inp: PathBuf,
    lang: String,
    offline: bool,
    cancellation: CancellationToken,
) -> i32 {
    match ocr_file_with_cancellation(&inp, &lang, offline, cancellation) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result.as_dict()).unwrap_or_else(|_| "{}".into())
            );
            0
        }
        Err(e) => {
            eprintln!("ocr error: {e}");
            1
        }
    }
}

pub(super) fn onnx(
    inp: PathBuf,
    model: PathBuf,
    input_name: Option<String>,
    layout: OnnxLayout,
    normalization: OnnxNormalization,
) -> i32 {
    let options = OnnxInferenceOptions {
        input_name,
        layout: layout.as_ml(),
        normalization: normalization.as_ml(),
    };
    let worker = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("onnx error: could not resolve the Open Scanline worker: {error}");
            return 1;
        }
    };
    match run_isolated_onnx_with_executable(&inp, &model, &options, &worker) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report.as_dict()).unwrap_or_else(|_| "{}".into())
            );
            0
        }
        Err(error) => {
            eprintln!("onnx error: {error}");
            1
        }
    }
}

pub(super) fn onnx_worker(
    inp: PathBuf,
    model: PathBuf,
    report: PathBuf,
    input_name: Option<String>,
    layout: OnnxLayout,
    normalization: OnnxNormalization,
    worker_protocol: String,
) -> i32 {
    let options = OnnxInferenceOptions {
        input_name,
        layout: layout.as_ml(),
        normalization: normalization.as_ml(),
    };
    match run_onnx_worker(&inp, &model, &report, &options, &worker_protocol) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("ONNX worker error: {error}");
            1
        }
    }
}

pub(super) struct BatchRequest {
    pub(super) config: AppConfig,
    pub(super) device: Option<String>,
    pub(super) allow_unlisted_escl: bool,
    pub(super) out_dir: PathBuf,
    pub(super) pages: Option<u32>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) seed: u32,
    pub(super) dpi: Option<u32>,
    pub(super) source: Option<ScanSource>,
    pub(super) duplex: Option<bool>,
    pub(super) format: Option<String>,
    pub(super) overrides: scan::PipelineOverrides,
    pub(super) auto_crop: Option<bool>,
    pub(super) auto_orient: Option<bool>,
    pub(super) invert: Option<bool>,
    pub(super) multipage_tiff: Option<PathBuf>,
    pub(super) multipage_pdf: Option<PathBuf>,
    pub(super) multipage_out: Option<PathBuf>,
    pub(super) export: crate::ExportOptions,
    pub(super) contact_sheet: Option<PathBuf>,
}

pub(super) fn batch(mut request: BatchRequest, cancellation: CancellationToken) -> i32 {
    let args = request.scan_args(&request.config);
    let requested_outputs = [
        args.multipage_tiff.clone(),
        args.multipage_pdf.clone(),
        args.multipage_out.clone(),
        args.contact_sheet.clone(),
    ];
    let result = run_batch_scan_with_export_options_and_token_and_policy(
        args,
        &request.export,
        cancellation,
        DeviceOpenPolicy {
            allow_unlisted_escl: request.allow_unlisted_escl,
        },
    );
    super::router::clear_pdf_password(&mut request.export.pdf_password);
    match result {
        Ok(paths) => {
            report_batch_outputs(&paths, &requested_outputs);
            0
        }
        Err(error) => {
            eprintln!("batch error: {error}");
            1
        }
    }
}

impl BatchRequest {
    fn scan_args(&self, config: &AppConfig) -> BatchScanArgs {
        let mut pipeline = scan::pipeline_with_overrides(config, &self.overrides);
        pipeline.invert = config.invert_colors;
        scan::apply_explicit_bool(&mut pipeline.auto_crop, self.auto_crop);
        scan::apply_explicit_bool(&mut pipeline.auto_orient, self.auto_orient);
        scan::apply_explicit_bool(&mut pipeline.invert, self.invert);
        let configured_format = self
            .format
            .clone()
            .unwrap_or_else(|| config.output_format.clone());
        let page_format = if configured_format.eq_ignore_ascii_case("pdf") {
            "png".to_string()
        } else {
            configured_format.clone()
        };
        let mut multipage_out = self.multipage_out.clone();
        if multipage_out.is_none() && self.multipage_pdf.is_none() && self.multipage_tiff.is_none()
        {
            let format = if configured_format.eq_ignore_ascii_case("pdf") {
                Some("pdf")
            } else if config.multipage {
                Some(config.multipage_format.as_str())
            } else {
                None
            };
            if let Some(format) = format {
                multipage_out = Some(
                    self.out_dir
                        .join(format!("{}_multipage.{}", config.output_name, format)),
                );
            }
        }
        BatchScanArgs {
            device: self
                .device
                .clone()
                .unwrap_or_else(|| config.last_device_id.clone()),
            out_dir: self.out_dir.clone(),
            pages: self.pages.unwrap_or(config.batch_pages.max(1)),
            width: self.width.unwrap_or(config.default_width),
            height: self.height.unwrap_or(config.default_height),
            seed: self.seed,
            dpi: self.dpi.unwrap_or(config.default_dpi),
            mode: self
                .source
                .map(ScanSource::mode)
                .unwrap_or(config.scan_mode),
            duplex: self.duplex.unwrap_or(config.duplex),
            format: page_format,
            multipage_tiff: self.multipage_tiff.clone(),
            multipage_pdf: self.multipage_pdf.clone(),
            multipage_out,
            contact_sheet: self.contact_sheet.clone().or_else(|| {
                config.contact_sheet.then(|| {
                    self.out_dir
                        .join(format!("{}_contact.bmp", config.output_name))
                })
            }),
            pipeline,
            on_progress: None,
        }
    }
}

fn report_batch_outputs(paths: &[PathBuf], requested_outputs: &[Option<PathBuf>; 4]) {
    for path in paths {
        print_wrote(path);
    }
    for path in requested_outputs.iter().flatten() {
        report_if_file(path);
    }
}

fn report_if_file(path: &Path) {
    if path.is_file() {
        print_wrote(path);
    }
}

pub(super) fn package(binary: PathBuf, out: PathBuf) -> i32 {
    match build_portable(&PackagingOptions { binary, out }) {
        Ok(path) => {
            print_wrote(&path);
            0
        }
        Err(error) => {
            eprintln!("package error: {error}");
            1
        }
    }
}

pub(super) fn help_text() -> i32 {
    print!("{}", user_manual_text());
    0
}
