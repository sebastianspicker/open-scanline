//! Native entry wrappers for the batch-capture workflow.

use crate::composition::Runtime;
use crate::domain::acquisition::DeviceOpenPolicy;
use crate::error::Result;
use crate::workflows::capture::batch::run_batch_scan_with_export_options_inner_with_ports;
pub use crate::workflows::capture::batch::{BatchCancelCheck, BatchScanArgs, MAX_BATCH_PAGES};
use crate::workflows::operation::CancellationToken;
use crate::workflows::publication::ExportOptions;
use std::path::PathBuf;

pub fn run_batch_scan(args: BatchScanArgs) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(
        args,
        &ExportOptions::default(),
        CancellationToken::new(),
    )
}

pub fn run_batch_scan_with_token(
    args: BatchScanArgs,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(args, &ExportOptions::default(), token)
}

pub fn run_batch_scan_with_export_options(
    args: BatchScanArgs,
    export: &ExportOptions,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(args, export, CancellationToken::new())
}

pub fn run_batch_scan_with_export_options_and_token(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token_and_policy(
        args,
        export,
        token,
        DeviceOpenPolicy::default(),
    )
}

pub fn run_batch_scan_with_export_options_and_token_and_policy(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_runtime(args, export, None, Some(token), policy)
}

pub fn run_batch_scan_with_export_options_and_cancel(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_runtime(
        args,
        export,
        cancel_check,
        None,
        DeviceOpenPolicy::default(),
    )
}

fn run_batch_scan_with_runtime(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<CancellationToken>,
    policy: DeviceOpenPolicy,
) -> Result<Vec<PathBuf>> {
    let runtime = Runtime::default();
    run_batch_scan_with_export_options_inner_with_ports(
        args,
        export,
        cancel_check,
        token,
        policy,
        runtime.acquisition(),
        runtime.media(),
    )
}
