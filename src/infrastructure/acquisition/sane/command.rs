use super::capabilities::SaneCapabilities;
use crate::domain::acquisition::{validate_scan_dpi, ScanMode, ScanRequest};
use crate::domain::image::PixelFormat;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::CommandSpec;
use std::path::Path;

/// Build an argument-vector invocation, never a shell string, so device names
/// and output paths are passed verbatim and can be asserted in tests.
pub fn scanimage_command(
    binary: &std::path::Path,
    sane_name: &str,
    request: &ScanRequest,
    output_path: &Path,
) -> CommandSpec {
    build_scanimage_command(binary, sane_name, request, output_path, None)
        .unwrap_or_else(|_| basic_scanimage_command(binary, sane_name, request, output_path))
}

fn basic_scanimage_command(
    binary: &std::path::Path,
    sane_name: &str,
    request: &ScanRequest,
    output_path: &Path,
) -> CommandSpec {
    let mut args = vec![
        "-d".into(),
        sane_name.into(),
        "--resolution".into(),
        request.dpi_x.to_string(),
        "--format=png".into(),
        format!("--output-file={}", output_path.display()),
    ];
    append_geometry(&mut args, request);
    CommandSpec {
        program: binary.display().to_string(),
        args,
    }
}

pub(super) fn build_scanimage_command(
    binary: &std::path::Path,
    sane_name: &str,
    request: &ScanRequest,
    output_path: &Path,
    capabilities: Option<&SaneCapabilities>,
) -> Result<CommandSpec> {
    validate_request(request)?;
    let mut args = vec!["-d".into(), sane_name.into()];
    append_resolution(&mut args, request, capabilities);
    let source_includes_duplex = append_source(&mut args, request, capabilities)?;
    append_color_mode(&mut args, request, capabilities)?;
    append_duplex(&mut args, request, capabilities, source_includes_duplex)?;
    args.extend([
        "--format=png".into(),
        format!("--output-file={}", output_path.display()),
    ]);
    append_geometry(&mut args, request);
    Ok(CommandSpec {
        program: binary.display().to_string(),
        args,
    })
}

fn validate_request(request: &ScanRequest) -> Result<()> {
    validate_dimensions(request)?;
    validate_scan_dpi(request.dpi_x, request.dpi_y)?;
    validate_region(request)
}

fn validate_dimensions(request: &ScanRequest) -> Result<()> {
    if request.width == 0 || request.height == 0 {
        return Err(ScanError::Invalid(
            "SANE dimensions must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_region(request: &ScanRequest) -> Result<()> {
    if let Some(region) = request.region {
        if invalid_region(region) {
            return Err(ScanError::Invalid(
                "SANE scan region must have non-negative offsets and positive size".into(),
            ));
        }
    }
    Ok(())
}

fn invalid_region(region: crate::domain::image::Rect) -> bool {
    region.x < 0 || region.y < 0 || region.width == 0 || region.height == 0
}

fn append_resolution(
    args: &mut Vec<String>,
    request: &ScanRequest,
    capabilities: Option<&SaneCapabilities>,
) {
    if let Some(capabilities) = capabilities {
        if capabilities.x_resolution.is_some() || capabilities.y_resolution.is_some() {
            if let Some(constraint) = &capabilities.x_resolution {
                args.extend([
                    "--x-resolution".into(),
                    constraint.closest_to(request.dpi_x).to_string(),
                ]);
            }
            if let Some(constraint) = &capabilities.y_resolution {
                args.extend([
                    "--y-resolution".into(),
                    constraint.closest_to(request.dpi_y).to_string(),
                ]);
            }
            return;
        }
        if let Some(constraint) = &capabilities.resolution {
            args.extend([
                "--resolution".into(),
                constraint.closest_to(request.dpi_x).to_string(),
            ]);
            return;
        }
    }
    args.extend(["--resolution".into(), request.dpi_x.to_string()]);
}

fn append_source(
    args: &mut Vec<String>,
    request: &ScanRequest,
    capabilities: Option<&SaneCapabilities>,
) -> Result<bool> {
    let Some(capabilities) = capabilities else {
        return append_default_source(args, request);
    };
    let Some(selected) = selected_source(capabilities, request)? else {
        return Ok(false);
    };
    let source_includes_duplex = selected.to_ascii_lowercase().contains("duplex");
    args.push(format!("--source={selected}"));
    Ok(source_includes_duplex)
}

fn append_default_source(args: &mut Vec<String>, request: &ScanRequest) -> Result<bool> {
    if request.mode == ScanMode::Reflective {
        return Ok(false);
    }
    args.push(format!("--source={}", fallback_source(request.mode)));
    Ok(false)
}

fn selected_source(
    capabilities: &SaneCapabilities,
    request: &ScanRequest,
) -> Result<Option<String>> {
    if !capabilities.source_option {
        return source_without_option(request);
    }
    if capabilities.source_values.is_empty() {
        return source_without_constraints(request);
    }
    let selected = matching_value(&capabilities.source_values, source_keywords(request))
        .or_else(|| duplex_fallback_source(capabilities, request))
        .map(str::to_string);
    selected
        .map(Some)
        .ok_or_else(|| unsupported_source(request))
}

fn source_without_option(request: &ScanRequest) -> Result<Option<String>> {
    if request.mode == ScanMode::Reflective {
        return Ok(None);
    }
    Err(ScanError::Unsupported(format!(
        "SANE device does not expose a source option for {} scanning",
        source_name(request.mode)
    )))
}

fn source_without_constraints(request: &ScanRequest) -> Result<Option<String>> {
    if request.mode == ScanMode::Reflective {
        return Ok(None);
    }
    Err(ScanError::Unsupported(format!(
        "SANE device advertises --source but its constraints cannot select {} scanning",
        source_name(request.mode)
    )))
}

fn source_keywords(request: &ScanRequest) -> &'static [&'static str] {
    match (request.mode, request.duplex) {
        (ScanMode::Document, true) => &["duplex", "adf-duplex", "adf duplex"],
        (ScanMode::Reflective, _) => &["flatbed", "platen", "reflective"],
        (ScanMode::Document, false) => &["automatic document feeder", "adf", "feeder", "document"],
        (ScanMode::Film, _) => &["transparency", "tpu", "film", "negative", "slide"],
    }
}

fn duplex_fallback_source<'a>(
    capabilities: &'a SaneCapabilities,
    request: &ScanRequest,
) -> Option<&'a str> {
    if request.mode != ScanMode::Document || !request.duplex || capabilities.duplex_option.is_none()
    {
        return None;
    }
    matching_value(
        &capabilities.source_values,
        &["automatic document feeder", "adf", "feeder", "document"],
    )
}

fn fallback_source(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Reflective => "Flatbed",
        ScanMode::Document => "Automatic Document Feeder",
        ScanMode::Film => "Transparency Unit",
    }
}

fn unsupported_source(request: &ScanRequest) -> ScanError {
    ScanError::Unsupported(format!(
        "SANE device does not advertise a {} source",
        source_name(request.mode)
    ))
}

fn source_name(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Reflective => "flatbed",
        ScanMode::Document => "document feeder",
        ScanMode::Film => "film/transparency",
    }
}

fn matching_value<'a>(values: &'a [String], keywords: &[&str]) -> Option<&'a str> {
    values.iter().find_map(|value| {
        let lower = value.to_ascii_lowercase();
        keywords
            .iter()
            .any(|keyword| lower.contains(keyword))
            .then_some(value.as_str())
    })
}

fn append_color_mode(
    args: &mut Vec<String>,
    request: &ScanRequest,
    capabilities: Option<&SaneCapabilities>,
) -> Result<()> {
    let selected = select_color_mode(request, capabilities)?;
    if let Some(selected) = selected {
        args.push(format!("--mode={selected}"));
    }
    Ok(())
}

fn select_color_mode<'a>(
    request: &ScanRequest,
    capabilities: Option<&'a SaneCapabilities>,
) -> Result<Option<&'a str>> {
    let Some(capabilities) = capabilities else {
        return Ok(Some(color_fallback(request)));
    };
    if !capabilities.mode_option {
        return Ok(None);
    }
    if capabilities.mode_values.is_empty() {
        return empty_mode_selection(request);
    }
    matching_value(&capabilities.mode_values, color_keywords(request))
        .map(Some)
        .ok_or_else(|| {
            ScanError::Unsupported(
                "SANE device does not advertise the requested pixel format".into(),
            )
        })
}

fn empty_mode_selection(request: &ScanRequest) -> Result<Option<&'static str>> {
    if request.pixel_format == PixelFormat::Rgb8 {
        return Ok(None);
    }
    Err(ScanError::Unsupported(
        "SANE device advertises --mode but its constraints cannot select the requested pixel format".into(),
    ))
}

fn color_keywords(request: &ScanRequest) -> &'static [&'static str] {
    match request.pixel_format {
        PixelFormat::Gray8 => &["gray", "grey", "grayscale", "greyscale"],
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => &["color", "colour", "rgb", "24 bit", "24bit"],
    }
}

fn color_fallback(request: &ScanRequest) -> &'static str {
    if request.pixel_format == PixelFormat::Gray8 {
        "Gray"
    } else {
        "Color"
    }
}

fn append_duplex(
    args: &mut Vec<String>,
    request: &ScanRequest,
    capabilities: Option<&SaneCapabilities>,
    source_includes_duplex: bool,
) -> Result<()> {
    if !request.duplex {
        return Ok(());
    }
    if request.mode != ScanMode::Document {
        return Err(ScanError::Invalid(
            "duplex is only valid with a document feeder source".into(),
        ));
    }
    if source_includes_duplex {
        return Ok(());
    }
    let option = capabilities
        .and_then(|value| value.duplex_option.as_deref())
        .unwrap_or("--duplex");
    if capabilities.is_some_and(|value| value.duplex_option.is_none()) {
        return Err(ScanError::Unsupported(
            "SANE device does not advertise duplex acquisition".into(),
        ));
    }
    args.push(format!("{option}=yes"));
    Ok(())
}

fn append_geometry(args: &mut Vec<String>, request: &ScanRequest) {
    let (left, top, width, height) = request
        .region
        .map(|region| {
            (
                region.x as u32,
                region.y as u32,
                region.width,
                region.height,
            )
        })
        .unwrap_or((0, 0, request.width, request.height));
    if request.region.is_some() {
        args.extend([
            "-l".into(),
            format!("{:.2}", left as f64 / request.dpi_x.max(1) as f64 * 25.4),
            "-t".into(),
            format!("{:.2}", top as f64 / request.dpi_y.max(1) as f64 * 25.4),
        ]);
    }
    if width > 0 && height > 0 {
        args.extend([
            "-x".into(),
            format!("{:.2}", width as f64 / request.dpi_x.max(1) as f64 * 25.4),
            "-y".into(),
            format!("{:.2}", height as f64 / request.dpi_y.max(1) as f64 * 25.4),
        ]);
    }
}
