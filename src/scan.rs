//! Compatibility facade for the single-capture workflow.

pub use crate::workflows::capture::single::ScanToFileArgs;
pub use crate::workflows::compat::{
    run_scan_to_file, run_scan_to_file_with_export_options,
    run_scan_to_file_with_export_options_and_token,
    run_scan_to_file_with_export_options_and_token_and_policy, run_scan_to_file_with_token,
};
