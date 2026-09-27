use super::archive::{write_binary_and_launchers, write_documentation_and_manifest};
use super::identity::{output_aliases_binary, valid_binary_name};
use super::verification::verify_archive;
use super::workspace::TemporaryOutput;
use super::{package_error, PackagingOptions};
use crate::error::{Result, ScanError};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub fn build_portable(options: &PackagingOptions) -> Result<PathBuf> {
    build_portable_with_hook(options, |_| {})
}

pub(super) fn build_portable_with_hook<F>(
    options: &PackagingOptions,
    mut on_snapshot_chunk: F,
) -> Result<PathBuf>
where
    F: FnMut(u64),
{
    let binary_name = validate_options(options)?;

    let (temporary, file) = TemporaryOutput::create(&options.out)?;

    let zip = ZipWriter::new(file);
    build_archive(zip, options, binary_name, &mut on_snapshot_chunk, temporary)
}

fn build_archive<F>(
    mut zip: ZipWriter<std::fs::File>,
    options: &PackagingOptions,
    binary_name: &str,
    on_snapshot_chunk: &mut F,
    temporary: TemporaryOutput,
) -> Result<PathBuf>
where
    F: FnMut(u64),
{
    let regular = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let binary_fingerprint = write_binary_and_launchers(
        &mut zip,
        &options.binary,
        binary_name,
        regular,
        on_snapshot_chunk,
    )?;
    write_documentation_and_manifest(
        &mut zip,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        binary_name,
        regular,
    )?;
    let file = zip
        .finish()
        .map_err(|error| package_error("could not finish portable archive", error))?;
    file.sync_all()?;
    drop(file);
    verify_archive(&temporary.path, binary_name, &binary_fingerprint)?;
    temporary.publish(&options.out)?;
    Ok(options.out.clone())
}

fn validate_options(options: &PackagingOptions) -> Result<&str> {
    validate_binary_exists(&options.binary)?;
    let name = binary_file_name(&options.binary)?;
    validate_binary_name(name)?;
    validate_distinct_output(options)?;
    Ok(name)
}
fn validate_binary_exists(binary: &Path) -> Result<()> {
    if binary.is_file() {
        Ok(())
    } else {
        Err(ScanError::Invalid(format!(
            "package binary does not exist: {}",
            binary.display()
        )))
    }
}
fn binary_file_name(binary: &Path) -> Result<&str> {
    binary
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ScanError::Invalid("package binary has no file name".into()))
}
fn validate_binary_name(name: &str) -> Result<()> {
    if valid_binary_name(name) {
        Ok(())
    } else {
        Err(ScanError::Invalid(
            "package binary file name must use only ASCII letters, digits, '.', '_' or '-'".into(),
        ))
    }
}
fn validate_distinct_output(options: &PackagingOptions) -> Result<()> {
    if output_aliases_binary(&options.binary, &options.out)? {
        Err(ScanError::Invalid(format!(
            "package output aliases package binary: {}",
            options.out.display()
        )))
    } else {
        Ok(())
    }
}
