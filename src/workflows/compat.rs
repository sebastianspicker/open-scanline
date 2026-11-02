//! Public default-option entries of the capture, batch, and process workflows.
//!
//! Each function here delegates to the canonical workflow entry
//! (`workflows::capture::{single, batch}` and `workflows::process`) with
//! default runtime options. The `crate::scan`, `crate::batch`, and
//! `crate::process` facades also export the canonical two-argument entries
//! as `*_with_options` together with their public options structs; callers
//! needing a token, export options, or a policy set them there.

use crate::error::Result;
use crate::workflows::capture::batch::{self, BatchCaptureOptions, BatchScanArgs};
use crate::workflows::capture::single::{self, CaptureOptions, ScanToFileArgs};
use crate::workflows::process::{self, ProcessOptions, ProcessRunOptions};
use std::path::PathBuf;

pub fn run_scan_to_file(args: ScanToFileArgs) -> Result<PathBuf> {
    single::run_scan_to_file(args, CaptureOptions::default())
}

pub fn run_batch_scan(args: BatchScanArgs) -> Result<Vec<PathBuf>> {
    batch::run_batch_scan(args, BatchCaptureOptions::default())
}

pub fn process_image_file(options: &ProcessOptions) -> Result<PathBuf> {
    process::process_image_file(options, ProcessRunOptions::default())
}
