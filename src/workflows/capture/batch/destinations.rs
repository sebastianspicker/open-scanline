//! Validation for batch page and aggregate output destinations.

use super::BatchScanArgs;
use crate::error::{Result, ScanError};
use crate::workflows::ports::media::MediaPort;
use std::path::{Path, PathBuf};

pub(super) fn validate_batch_destinations<M: MediaPort>(
    args: &BatchScanArgs,
    file_ext: &str,
    media: &M,
) -> Result<()> {
    let page_paths = generated_page_paths(args, file_ext, media)?;
    let destinations = requested_destinations(args, media)?;
    validate_destination_aliases(&destinations, &page_paths, media)
}

fn generated_page_paths<M: MediaPort>(
    args: &BatchScanArgs,
    file_ext: &str,
    media: &M,
) -> Result<Vec<PathBuf>> {
    (1..=args.pages)
        .map(|page| {
            let path = args.out_dir.join(format!("page_{page:03}.{file_ext}"));
            media.validate_output_leaf(&path, &format!("generated page {page}"))?;
            Ok(path)
        })
        .collect()
}

fn requested_destinations<M: MediaPort>(
    args: &BatchScanArgs,
    media: &M,
) -> Result<Vec<(&'static str, PathBuf)>> {
    let mut destinations = Vec::new();
    add_multipage_output(args, media, &mut destinations)?;
    add_named_output(
        args.multipage_tiff.as_deref(),
        "multipage TIFF",
        &["tif", "tiff"],
        media,
        &mut destinations,
    )?;
    add_named_output(
        args.multipage_pdf.as_deref(),
        "multipage PDF",
        &["pdf"],
        media,
        &mut destinations,
    )?;
    add_contact_sheet(args, media, &mut destinations)?;
    Ok(destinations)
}

fn add_multipage_output<M: MediaPort>(
    args: &BatchScanArgs,
    media: &M,
    destinations: &mut Vec<(&'static str, PathBuf)>,
) -> Result<()> {
    let Some(path) = args.multipage_out.as_deref() else {
        return Ok(());
    };
    validate_multipage_output_path(path, media)?;
    let effective = if path.extension().is_none() {
        path.with_extension("tif")
    } else {
        path.to_path_buf()
    };
    media.validate_output_leaf(&effective, "multipage output")?;
    destinations.push(("multipage output", effective));
    Ok(())
}

fn add_named_output<M: MediaPort>(
    path: Option<&Path>,
    label: &'static str,
    allowed: &[&str],
    media: &M,
    destinations: &mut Vec<(&'static str, PathBuf)>,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    validate_named_container_path(path, label, allowed, media)?;
    destinations.push((label, path.to_path_buf()));
    Ok(())
}

fn add_contact_sheet<M: MediaPort>(
    args: &BatchScanArgs,
    media: &M,
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
    validate_image_output_path(&effective, "contact sheet", media)?;
    destinations.push(("contact sheet", effective));
    Ok(())
}

fn validate_destination_aliases<M: MediaPort>(
    destinations: &[(&str, PathBuf)],
    page_paths: &[PathBuf],
    media: &M,
) -> Result<()> {
    for index in 0..destinations.len() {
        let (label, path) = &destinations[index];
        validate_container_aliases(label, path, &destinations[..index], media)?;
        validate_page_aliases(label, path, page_paths, media)?;
    }
    Ok(())
}

fn validate_container_aliases<M: MediaPort>(
    label: &str,
    path: &Path,
    previous: &[(&str, PathBuf)],
    media: &M,
) -> Result<()> {
    for (other_label, other_path) in previous {
        if media.output_paths_alias(path, other_path)? {
            return Err(ScanError::Invalid(format!(
                "{label} destination aliases {other_label}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_page_aliases<M: MediaPort>(
    label: &str,
    path: &Path,
    page_paths: &[PathBuf],
    media: &M,
) -> Result<()> {
    for (page_index, page_path) in page_paths.iter().enumerate() {
        if media.output_paths_alias(path, page_path)? {
            let page = page_index + 1;
            return Err(ScanError::Invalid(format!(
                "{label} destination aliases generated page {page}: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_multipage_output_path<M: MediaPort>(path: &Path, media: &M) -> Result<()> {
    if path.extension().is_none() {
        return Ok(());
    }
    validate_named_container_path(path, "multipage output", &["pdf", "tif", "tiff"], media)
}

fn validate_named_container_path<M: MediaPort>(
    path: &Path,
    label: &str,
    allowed: &[&str],
    media: &M,
) -> Result<()> {
    media.validate_output_leaf(path, label)?;
    let extension = output_extension(path, label)?;
    if !allowed.contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

fn validate_image_output_path<M: MediaPort>(path: &Path, label: &str, media: &M) -> Result<()> {
    media.validate_output_leaf(path, label)?;
    let extension = output_extension(path, label)?;
    if !media.supports_extension(&extension) {
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
