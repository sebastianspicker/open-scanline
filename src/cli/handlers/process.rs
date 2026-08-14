use super::{common::print_wrote, scan};
use crate::config::AppConfig;
use crate::device::CancellationToken;
use crate::process::{process_image_file_with_export_options_and_token, ProcessOptions};
use std::path::PathBuf;

pub(super) struct Request {
    pub(super) config: AppConfig,
    pub(super) inp: PathBuf,
    pub(super) out: PathBuf,
    pub(super) export: crate::ExportOptions,
    pub(super) overrides: scan::PipelineOverrides,
    pub(super) invert: Option<bool>,
    pub(super) auto_crop: Option<bool>,
    pub(super) auto_orient: Option<bool>,
    pub(super) quality: Option<u8>,
}

pub(super) fn run(mut request: Request, cancellation: CancellationToken) -> i32 {
    let mut pipeline = scan::pipeline_with_overrides(&request.config, &request.overrides);
    pipeline.invert = request.config.invert_colors;
    scan::apply_explicit_bool(&mut pipeline.invert, request.invert);
    scan::apply_explicit_bool(&mut pipeline.auto_crop, request.auto_crop);
    scan::apply_explicit_bool(&mut pipeline.auto_orient, request.auto_orient);
    let result = process_image_file_with_export_options_and_token(
        &ProcessOptions {
            src: request.inp,
            dst: request.out,
            pipeline,
            quality: request.quality,
        },
        &request.export,
        cancellation,
    );
    super::router::clear_pdf_password(&mut request.export.pdf_password);
    match result {
        Ok(path) => finish_processing(path),
        Err(error) => report_process_error(error),
    }
}

fn finish_processing(path: PathBuf) -> i32 {
    print_wrote(&path);
    0
}

fn report_process_error(error: crate::core::ScanError) -> i32 {
    eprintln!("process error: {error}");
    1
}
