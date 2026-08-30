//! Native entry wrappers for the single-capture workflow.

use crate::composition::Runtime;
use crate::domain::acquisition::DeviceOpenPolicy;
use crate::error::Result;
use crate::workflows::capture::single::run_scan_to_file_with_export_options_and_token_and_policy_with_ports;
pub use crate::workflows::capture::single::ScanToFileArgs;
use crate::workflows::operation::CancellationToken;
use crate::workflows::publication::ExportOptions;
use std::path::PathBuf;

pub fn run_scan_to_file(args: ScanToFileArgs) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(
        args,
        &ExportOptions::default(),
        CancellationToken::new(),
    )
}

pub fn run_scan_to_file_with_token(
    args: ScanToFileArgs,
    token: CancellationToken,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(args, &ExportOptions::default(), token)
}

pub fn run_scan_to_file_with_export_options(
    args: ScanToFileArgs,
    export: &ExportOptions,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token(args, export, CancellationToken::new())
}

pub fn run_scan_to_file_with_export_options_and_token(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    run_scan_to_file_with_export_options_and_token_and_policy(
        args,
        export,
        token,
        DeviceOpenPolicy::default(),
    )
}

pub fn run_scan_to_file_with_export_options_and_token_and_policy(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<PathBuf> {
    let runtime = Runtime::default();
    run_scan_to_file_with_export_options_and_token_and_policy_with_ports(
        args,
        export,
        token,
        policy,
        runtime.acquisition(),
        runtime.media(),
    )
}
