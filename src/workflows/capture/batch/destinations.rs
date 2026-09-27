//! Validation for batch page and aggregate output destinations.

use super::BatchScanArgs;
use crate::error::{Result, ScanError};
use std::path::{Path, PathBuf};

pub(super) fn validate_batch_destinations(args: &BatchScanArgs, file_ext: &str) -> Result<()> {
    let page_paths = generated_page_paths(args, file_ext)?;
    let destinations = requested_destinations(args)?;
    validate_destination_aliases(&destinations, &page_paths)
}

fn generated_page_paths(args: &BatchScanArgs, file_ext: &str) -> Result<Vec<PathBuf>> {
    (1..=args.pages)
        .map(|page| {
            let path = args.out_dir.join(format!("page_{page:03}.{file_ext}"));
            crate::infrastructure::runtime::atomic_publish::validate_output_leaf(
                &path,
                &format!("generated page {page}"),
            )?;
            Ok(path)
        })
        .collect()
}

fn requested_destinations(args: &BatchScanArgs) -> Result<Vec<(&'static str, PathBuf)>> {
    let mut destinations = Vec::new();
    add_multipage_output(args, &mut destinations)?;
    add_named_output(
        args.multipage_tiff.as_deref(),
        "multipage TIFF",
        &["tif", "tiff"],
        &mut destinations,
    )?;
    add_named_output(
        args.multipage_pdf.as_deref(),
        "multipage PDF",
        &["pdf"],
        &mut destinations,
    )?;
    add_contact_sheet(args, &mut destinations)?;
    Ok(destinations)
}

fn add_multipage_output(
    args: &BatchScanArgs,
    destinations: &mut Vec<(&'static str, PathBuf)>,
) -> Result<()> {
    let Some(path) = args.multipage_out.as_deref() else {
        return Ok(());
    };
    validate_multipage_output_path(path)?;
    let effective = if path.extension().is_none() {
        path.with_extension("tif")
    } else {
        path.to_path_buf()
    };
    crate::infrastructure::runtime::atomic_publish::validate_output_leaf(
        &effective,
        "multipage output",
    )?;
    destinations.push(("multipage output", effective));
    Ok(())
}

fn add_named_output(
    path: Option<&Path>,
    label: &'static str,
    allowed: &[&str],
    destinations: &mut Vec<(&'static str, PathBuf)>,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    validate_named_container_path(path, label, allowed)?;
    destinations.push((label, path.to_path_buf()));
    Ok(())
}

fn add_contact_sheet(
    args: &BatchScanArgs,
    destinations: &mut Vec<(&'static str, PathBuf)>,
) -> Result<()> {
    let Some(path) = args.contact_sheet.as_deref() else {
        return Ok(());
    };
    let effective = if path.extension().is_none() {
        path.with_extension("bmp")
    } else {
        path.to_path_buf()
    };
    validate_image_output_path(&effective, "contact sheet")?;
    destinations.push(("contact sheet", effective));
    Ok(())
}

fn validate_destination_aliases(
    destinations: &[(&str, PathBuf)],
    page_paths: &[PathBuf],
) -> Result<()> {
    for index in 0..destinations.len() {
        let (label, path) = &destinations[index];
        validate_container_aliases(label, path, &destinations[..index])?;
        validate_page_aliases(label, path, page_paths)?;
    }
    Ok(())
}

fn validate_container_aliases(
    label: &str,
    path: &Path,
    previous: &[(&str, PathBuf)],
) -> Result<()> {
    for (other_label, other_path) in previous {
        if crate::infrastructure::runtime::atomic_publish::output_paths_alias(path, other_path)? {
            return Err(ScanError::Invalid(format!(
                "{label} destination aliases {other_label}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_page_aliases(label: &str, path: &Path, page_paths: &[PathBuf]) -> Result<()> {
    for (page_index, page_path) in page_paths.iter().enumerate() {
        if crate::infrastructure::runtime::atomic_publish::output_paths_alias(path, page_path)? {
            let page = page_index + 1;
            return Err(ScanError::Invalid(format!(
                "{label} destination aliases generated page {page}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_multipage_output_path(path: &Path) -> Result<()> {
    if path.extension().is_none() {
        return Ok(());
    }
    validate_named_container_path(path, "multipage output", &["pdf", "tif", "tiff"])
}

fn validate_named_container_path(path: &Path, label: &str, allowed: &[&str]) -> Result<()> {
    crate::infrastructure::runtime::atomic_publish::validate_output_leaf(path, label)?;
    let extension = output_extension(path, label)?;
    if !allowed.contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

fn validate_image_output_path(path: &Path, label: &str) -> Result<()> {
    crate::infrastructure::runtime::atomic_publish::validate_output_leaf(path, label)?;
    let extension = output_extension(path, label)?;
    if !crate::infrastructure::media::supported_extensions().contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

fn output_extension(path: &Path, label: &str) -> Result<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid(format!("{label} has no supported extension")))
}
