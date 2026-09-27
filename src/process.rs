//! Compatibility facade for the file-processing workflow.

pub use crate::workflows::compat::{
    process_image_file, process_image_file_with_export_options,
    process_image_file_with_export_options_and_token, process_image_file_with_token,
};
pub use crate::workflows::process::ProcessOptions;
