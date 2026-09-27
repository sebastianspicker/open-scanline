//! Portable application packaging.

mod archive;
mod build;
mod identity;
mod verification;
mod workspace;

use crate::error::ScanError;
use std::path::PathBuf;

const PORTABLE_ROOT: &str = "open-scanline";
const PROJECT_README: &[u8] = include_bytes!("../../../README.md");
const PROJECT_LICENSE: &[u8] = include_bytes!("../../../LICENSE");
const THIRD_PARTY_NOTICES: &[u8] = include_bytes!("../../../THIRD_PARTY_NOTICES.md");
const RUST_DEPENDENCY_LICENSES: &[u8] =
    include_bytes!("../../../assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md");
const RUST_DEPENDENCY_LICENSES_NAME: &str = "licenses/RUST_DEPENDENCIES_ALL_FEATURES.md";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagingOptions {
    pub binary: PathBuf,
    pub out: PathBuf,
}

pub use build::build_portable;

fn package_error(context: &str, error: impl std::fmt::Display) -> ScanError {
    ScanError::Other(format!("{context}: {error}"))
}
