//! Public permutations of the capture, batch, and process workflows.
//!
//! Each function here is a thin, one-line delegation to the canonical
//! workflow entry (`workflows::capture::{single, batch}` and
//! `workflows::process`) built from a runtime options struct. This module
//! preserves the documented public API surface exposed through the
//! `crate::scan`, `crate::batch`, and `crate::process` facades, and is the
//! only place these argument permutations exist; new crate-internal call
//! sites should call the canonical entries directly instead of adding more
//! permutations here.

use crate::domain::acquisition::DeviceOpenPolicy;
use crate::domain::export::ExportOptions;
use crate::error::Result;
use crate::operation::CancellationToken;
use crate::workflows::capture::batch::{
    self, BatchCancelCheck, BatchCaptureOptions, BatchScanArgs,
};
use crate::workflows::capture::single::{self, CaptureOptions, ScanToFileArgs};
use crate::workflows::process::{self, ProcessOptions, ProcessRunOptions};
use std::path::PathBuf;

pub fn run_scan_to_file(args: ScanToFileArgs) -> Result<PathBuf> {
    single::run_scan_to_file(args, CaptureOptions::default())
}

pub fn run_scan_to_file_with_token(
    args: ScanToFileArgs,
    token: CancellationToken,
) -> Result<PathBuf> {
    single::run_scan_to_file(
        args,
        CaptureOptions {
            cancellation: token,
            ..CaptureOptions::default()
        },
    )
}

pub fn run_scan_to_file_with_export_options(
    args: ScanToFileArgs,
    export: &ExportOptions,
) -> Result<PathBuf> {
    single::run_scan_to_file(
        args,
        CaptureOptions {
            export: export.clone(),
            ..CaptureOptions::default()
        },
    )
}

pub fn run_scan_to_file_with_export_options_and_token(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    single::run_scan_to_file(
        args,
        CaptureOptions {
            export: export.clone(),
            cancellation: token,
            ..CaptureOptions::default()
        },
    )
}

pub fn run_scan_to_file_with_export_options_and_token_and_policy(
    args: ScanToFileArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<PathBuf> {
    single::run_scan_to_file(
        args,
        CaptureOptions {
            export: export.clone(),
            cancellation: token,
            policy,
        },
    )
}

pub fn run_batch_scan(args: BatchScanArgs) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(args, BatchCaptureOptions::default())
}

pub fn run_batch_scan_with_token(
    args: BatchScanArgs,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(
        args,
        BatchCaptureOptions {
            token: Some(token),
            ..BatchCaptureOptions::default()
        },
    )
}

pub fn run_batch_scan_with_export_options(
    args: BatchScanArgs,
    export: &ExportOptions,
) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(
        args,
        BatchCaptureOptions {
            export: export.clone(),
            ..BatchCaptureOptions::default()
        },
    )
}

pub fn run_batch_scan_with_export_options_and_token(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(
        args,
        BatchCaptureOptions {
            export: export.clone(),
            token: Some(token),
            ..BatchCaptureOptions::default()
        },
    )
}

pub fn run_batch_scan_with_export_options_and_token_and_policy(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(
        args,
        BatchCaptureOptions {
            export: export.clone(),
            token: Some(token),
            policy,
            ..BatchCaptureOptions::default()
        },
    )
}

pub fn run_batch_scan_with_export_options_and_cancel(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(
        args,
        BatchCaptureOptions {
            export: export.clone(),
            cancel_check,
            ..BatchCaptureOptions::default()
        },
    )
}

pub fn process_image_file(options: &ProcessOptions) -> Result<PathBuf> {
    process::process_image_file(options, ProcessRunOptions::default())
}

pub fn process_image_file_with_token(
    options: &ProcessOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process::process_image_file(
        options,
        ProcessRunOptions {
            cancellation: Some(token),
            ..ProcessRunOptions::default()
        },
    )
}

pub fn process_image_file_with_export_options(
    options: &ProcessOptions,
    export: &ExportOptions,
) -> Result<PathBuf> {
    process::process_image_file(
        options,
        ProcessRunOptions {
            export: export.clone(),
            ..ProcessRunOptions::default()
        },
    )
}

pub fn process_image_file_with_export_options_and_token(
    options: &ProcessOptions,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process::process_image_file(
        options,
        ProcessRunOptions {
            export: export.clone(),
            cancellation: Some(token),
        },
    )
}
