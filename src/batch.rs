//! Compatibility facade for the batch-capture workflow.

pub use crate::workflows::capture::batch::{BatchCancelCheck, BatchScanArgs, MAX_BATCH_PAGES};
pub use crate::workflows::compat::{
    run_batch_scan, run_batch_scan_with_export_options,
    run_batch_scan_with_export_options_and_cancel, run_batch_scan_with_export_options_and_token,
    run_batch_scan_with_export_options_and_token_and_policy, run_batch_scan_with_token,
};
