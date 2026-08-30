//! Crate-wide operational error contract.

use thiserror::Error;

/// Unified application error type.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("cancelled: {0}")]
    Cancelled(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(String),
    #[error("{0}")]
    Other(String),
}

impl From<image::ImageError> for ScanError {
    fn from(error: image::ImageError) -> Self {
        Self::Image(error.to_string())
    }
}

impl From<serde_json::Error> for ScanError {
    fn from(error: serde_json::Error) -> Self {
        Self::Other(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ScanError>;
