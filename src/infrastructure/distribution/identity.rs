use super::package_error;
use crate::error::Result;
use crate::infrastructure::runtime::atomic_publish::files_have_same_identity;
use std::fs;
use std::path::Path;

pub(super) fn valid_binary_name(binary_name: &str) -> bool {
    !binary_name.is_empty()
        && binary_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(super) fn output_aliases_binary(binary: &Path, out: &Path) -> Result<bool> {
    match fs::metadata(out) {
        Ok(_) => {
            if files_have_same_identity(binary, out)
                .map_err(|error| package_error("could not inspect package output", error))?
            {
                return Ok(true);
            }
            Ok(fs::canonicalize(binary)? == fs::canonicalize(out)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(package_error("could not inspect package output", error)),
    }
}
