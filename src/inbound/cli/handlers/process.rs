use super::{
    common::{print_verbose_hint, print_wrote},
    scan,
};
use crate::infrastructure::config::AppConfig;
use crate::operation::CancellationToken;
use crate::workflows::process::{self, ProcessOptions, ProcessRunOptions};
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
    pub(super) overwrite: bool,
}

pub(super) fn run(request: Request, cancellation: CancellationToken) -> i32 {
    let mut pipeline = scan::pipeline_with_overrides(&request.config, &request.overrides);
    pipeline.invert = request.config.invert_colors;
    scan::apply_explicit_bool(&mut pipeline.invert, request.invert);
    scan::apply_explicit_bool(&mut pipeline.auto_crop, request.auto_crop);
    scan::apply_explicit_bool(&mut pipeline.auto_orient, request.auto_orient);
    let result = process::process_image_file(
        &ProcessOptions {
            src: request.inp,
            dst: request.out,
            pipeline,
            quality: request.quality,
            overwrite: request.overwrite,
        },
        ProcessRunOptions {
            export: request.export,
            cancellation: Some(cancellation),
        },
    );
    match result {
        Ok(path) => {
            print_wrote(&path);
            0
        }
        Err(error) => report_process_error(error),
    }
}

fn report_process_error(error: crate::error::ScanError) -> i32 {
    eprintln!("process error: {error}");
    print_verbose_hint();
    1
}
