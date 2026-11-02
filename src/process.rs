//! Compatibility facade for the file-processing workflow.

pub use crate::workflows::compat::process_image_file;
pub use crate::workflows::process::{
    process_image_file as process_image_file_with_options, ProcessOptions, ProcessRunOptions,
};
