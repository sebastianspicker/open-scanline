//! Compatibility facade for the historical OSL-CORE path.

pub use crate::domain::acquisition::{ScanMode, ScanProgress, ScanRequest, MIN_SCAN_DPI};
pub use crate::domain::image::{
    ImageBuffer, PixelFormat, Rect, Rotate, MAX_IMAGE_BYTES, MAX_IMAGE_DIMENSION,
};
pub use crate::domain::processing::PipelinePrefs;
pub use crate::error::{Result, ScanError};
