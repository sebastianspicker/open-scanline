//! Compatibility facade for the batch-capture workflow.

pub use crate::workflows::capture::batch::{
    run_batch_scan as run_batch_scan_with_options, BatchCancelCheck, BatchCaptureOptions,
    BatchScanArgs, MAX_BATCH_PAGES,
};
pub use crate::workflows::compat::run_batch_scan;
