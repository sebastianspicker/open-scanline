use super::capabilities::SaneCapabilities;
use crate::domain::acquisition::{validate_scan_dpi, ScanMode, ScanRequest};
use crate::domain::image::PixelFormat;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::CommandSpec;
use std::path::Path;

/// Build an argument-vector invocation, never a shell string, so device names
/// and output paths are passed verbatim and can be asserted in tests.
pub(super) fn build_scanimage_command(
    binary: &std::path::Path,
    sane_name: &str,
    request: &ScanRequest,
    output_path: &Path,
    capabilities: Option<&SaneCapabilities>,
) -> Result<CommandSpec> {
    validate_request(request)?;
    let mut args = vec!["-d".into(), sane_name.into()];
    // Some drivers reset the resolution when the source is set.
    let source_includes_duplex = append_source(&mut args, request, capabilities)?;
    append_resolution(&mut args, request, capabilities);
    append_color_mode(&mut args, request, capabilities)?;
    append_duplex(&mut args, request, capabilities, source_includes_duplex)?;
    args.extend([
        "--format=png".into(),
        format!("--output-file={}", output_path.display()),
    ]);
    append_geometry(&mut args, request, capabilities);
    Ok(CommandSpec {
        program: binary.display().to_string(),
        args,
    })
}

fn validate_request(request: &ScanRequest) -> Result<()> {
    validate_scan_dpi(request.dpi_x, request.dpi_y)?;
    validate_region(request)
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
        let (x, y) = (&capabilities.x_resolution, &capabilities.y_resolution);
        if let (Some(x), Some(y)) = (x, y) {
            args.extend([
                "--x-resolution".into(),
                x.closest_to(request.dpi_x).to_string(),
                "--y-resolution".into(),
                y.closest_to(request.dpi_y).to_string(),
            ]);
            return;
        }
        let single = capabilities
            .resolution
            .as_ref()
            .map(|constraint| ("--resolution", constraint))
            .or_else(|| x.as_ref().map(|constraint| ("--x-resolution", constraint)));
        if let Some((option, constraint)) = single {
            args.extend([
                option.into(),
                constraint.closest_to(request.dpi_x).to_string(),
            ]);
        }
        if let Some(constraint) = y {
            args.extend([
                "--y-resolution".into(),
                constraint.closest_to(request.dpi_y).to_string(),
            ]);
        }
        if single.is_some() || y.is_some() {
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

pub(super) fn selected_source(
    capabilities: &SaneCapabilities,
    request: &ScanRequest,
) -> Result<Option<String>> {
    if !capabilities.source_option {
        return source_without_option(request);
    }
    if capabilities.source_values.is_empty() {
        return source_without_constraints(request);
    }
    let selected = simplex_feeder_source(capabilities, request)
        .or_else(|| {
            capabilities
                .source_values
                .iter()
                .map(String::as_str)
                .find(|value| {
                    let document = request.mode == ScanMode::Document;
                    // A Document request must not fall back to the flatbed, and a
                    // simplex one must not get a source that returns two sides.
                    let excluded = document
                        && (is_flatbed(value) || (!request.duplex && !is_single_sided(value)));
                    !excluded
                        && matching_value(&[value.to_string()], source_keywords(request)).is_some()
                })
        })
        .or_else(|| duplex_fallback_source(capabilities, request))
        .or_else(|| feeder_only_source(capabilities, request))
        .map(str::to_string);
    selected
        .map(Some)
        .ok_or_else(|| unsupported_source(request, &capabilities.source_values))
}

/// A single-sided feeder request must not pick "ADF Duplex" or "ADF Back"
/// just because the device lists it first.
fn simplex_feeder_source<'a>(
    capabilities: &'a SaneCapabilities,
    request: &ScanRequest,
) -> Option<&'a str> {
    if request.mode != ScanMode::Document || request.duplex {
        return None;
    }
    capabilities
        .source_values
        .iter()
        .filter(|value| {
            !is_flatbed(value)
                && matching_value(std::slice::from_ref(value), source_keywords(request)).is_some()
        })
        .find(|value| is_single_sided(value))
        .map(String::as_str)
}

fn is_single_sided(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !lower.contains("duplex") && !lower.contains("back")
}

const FEEDER_KEYWORDS: &[&str] = &["automatic document feeder", "adf", "feeder", "document"];
const FLATBED_KEYWORDS: &[&str] = &["flatbed", "platen", "reflective", "document table"];

/// "Document Table" also contains the feeder keyword "document".
fn is_flatbed(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    FLATBED_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
}

/// True when the device advertises a feeder and no flatbed or platen source.
/// Other sources (canon_dr lists Card sources) don't make it a flatbed.
pub(super) fn feeder_only(capabilities: &SaneCapabilities) -> bool {
    let values = &capabilities.source_values;
    matching_value(values, FEEDER_KEYWORDS).is_some()
        && matching_value(values, FLATBED_KEYWORDS).is_none()
}

/// A sheet-fed device has no flatbed, so the flatbed default would always
/// fail. When no advertised source is a flatbed, use the single-sided feeder.
fn feeder_only_source<'a>(
    capabilities: &'a SaneCapabilities,
    request: &ScanRequest,
) -> Option<&'a str> {
    if request.mode != ScanMode::Reflective || !feeder_only(capabilities) {
        return None;
    }
    let feeders = || {
        capabilities
            .source_values
            .iter()
            .filter(|value| matching_value(std::slice::from_ref(value), FEEDER_KEYWORDS).is_some())
    };
    feeders()
        .find(|value| is_single_sided(value))
        .map(String::as_str)
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
        (ScanMode::Reflective, _) => FLATBED_KEYWORDS,
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
    // "Document Table" is Epson's flatbed; it must never satisfy a feeder request.
    capabilities
        .source_values
        .iter()
        .filter(|value| !is_flatbed(value))
        .find(|value| {
            matching_value(
                std::slice::from_ref(*value),
                &["automatic document feeder", "adf", "feeder", "document"],
            )
            .is_some()
        })
        .map(String::as_str)
}

fn fallback_source(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Reflective => "Flatbed",
        ScanMode::Document => "Automatic Document Feeder",
        ScanMode::Film => "Transparency Unit",
    }
}

fn unsupported_source(request: &ScanRequest, advertised: &[String]) -> ScanError {
    ScanError::Unsupported(format!(
        "SANE device does not advertise a {} source; it offers: {}",
        source_name(request.mode),
        advertised.join(", ")
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
    let (values, keywords) = (&capabilities.mode_values, color_keywords(request));
    // Epson lists "Gray[Error Diffusion]" (dithered) before "True Gray"; a plain value wins.
    values
        .iter()
        .filter(|value| !value.contains('['))
        .find_map(|value| matching_value(std::slice::from_ref(value), keywords))
        .or_else(|| matching_value(values, keywords))
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

fn append_geometry(
    args: &mut Vec<String>,
    request: &ScanRequest,
    capabilities: Option<&SaneCapabilities>,
) {
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
    // A zero axis keeps the device default, which is its maximum extent.
    // Sheet-fed backends clamp -x/-y to the page size, and scanimage
    // range-checks -x/-y while parsing, so the page size must come first.
    if width > 0 {
        let millimetres = format!("{:.2}", width as f64 / request.dpi_x.max(1) as f64 * 25.4);
        if capabilities.is_some_and(|value| value.page_width_option) {
            args.push(format!("--page-width={millimetres}"));
        }
        args.extend(["-x".into(), millimetres]);
    }
    if height > 0 {
        let millimetres = format!("{:.2}", height as f64 / request.dpi_y.max(1) as f64 * 25.4);
        if capabilities.is_some_and(|value| value.page_height_option) {
            args.push(format!("--page-height={millimetres}"));
        }
        args.extend(["-y".into(), millimetres]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::acquisition::sane::capabilities::parse_scanimage_help;

    fn command_args(request: &ScanRequest, capabilities: Option<&SaneCapabilities>) -> Vec<String> {
        build_scanimage_command(
            Path::new("scanimage"),
            "dev",
            request,
            Path::new("out.png"),
            capabilities,
        )
        .unwrap()
        .args
    }

    #[test]
    fn feeder_only_device_scans_from_its_front_feeder_by_default() {
        let capabilities =
            parse_scanimage_help("    --source ADF Front|ADF Back|ADF Duplex [ADF Front]\n");
        let args = command_args(&ScanRequest::default(), Some(&capabilities));
        assert!(args.contains(&"--source=ADF Front".to_string()));

        let film = ScanRequest {
            mode: ScanMode::Film,
            ..ScanRequest::default()
        };
        let error = build_scanimage_command(
            Path::new("scanimage"),
            "dev",
            &film,
            Path::new("out.png"),
            Some(&capabilities),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("offers: ADF Front, ADF Back, ADF Duplex"),
            "{error}"
        );
    }

    #[test]
    fn single_sided_feeder_requests_skip_duplex_and_back_sources() {
        let capabilities =
            parse_scanimage_help("    --source ADF Duplex|ADF Back|ADF Front [ADF Duplex]\n");
        for mode in [ScanMode::Document, ScanMode::Reflective] {
            let request = ScanRequest {
                mode,
                ..ScanRequest::default()
            };
            let args = command_args(&request, Some(&capabilities));
            assert!(
                args.contains(&"--source=ADF Front".to_string()),
                "{mode:?}: {args:?}"
            );
        }
    }

    #[test]
    fn single_sided_requests_never_fall_back_to_a_duplex_only_feeder() {
        let capabilities = parse_scanimage_help("    --source ADF Duplex|ADF Back [ADF Duplex]\n");
        for mode in [ScanMode::Document, ScanMode::Reflective] {
            let request = ScanRequest {
                mode,
                ..ScanRequest::default()
            };
            let result = build_scanimage_command(
                Path::new("scanimage"),
                "dev",
                &request,
                Path::new("out.png"),
                Some(&capabilities),
            );
            assert!(
                result.is_err(),
                "{mode:?}: {:?}",
                result.map(|spec| spec.args)
            );
        }
    }

    #[test]
    fn gray_requests_prefer_a_plain_gray_mode_over_a_dithered_one() {
        let capabilities = parse_scanimage_help(
            "    --mode Black & White|Gray[Error Diffusion]|True Gray|24bit Color [24bit Color]\n",
        );
        let request = ScanRequest {
            pixel_format: PixelFormat::Gray8,
            ..ScanRequest::default()
        };
        let args = command_args(&request, Some(&capabilities));
        assert!(args.contains(&"--mode=True Gray".to_string()), "{args:?}");
    }

    #[test]
    fn inactive_options_are_not_advertised() {
        let capabilities = parse_scanimage_help(
            "    --source ADF Front|ADF Duplex [ADF Front]\n    --duplex[=(yes|no)] [inactive]\n    --page-width 0..215.9mm [215.9] [inactive]\n",
        );
        assert!(capabilities.duplex_option.is_none());
        assert!(!capabilities.page_width_option);
        assert!(capabilities.source_option);
    }

    #[test]
    fn card_sources_do_not_make_a_feeder_a_flatbed() {
        let capabilities = parse_scanimage_help(
            "    --source ADF Front|ADF Back|ADF Duplex|Card Front|Card Back|Card Duplex [ADF Front]\n",
        );
        assert!(feeder_only(&capabilities));
        let args = command_args(&ScanRequest::default(), Some(&capabilities));
        assert!(args.contains(&"--source=ADF Front".to_string()), "{args:?}");

        let with_flatbed =
            parse_scanimage_help("    --source Flatbed|ADF Front|ADF Duplex [Flatbed]\n");
        assert!(!feeder_only(&with_flatbed));

        let epson = parse_scanimage_help(
            "    --source Document Table|Automatic Document Feeder [Document Table]\n",
        );
        assert!(!feeder_only(&epson));
    }

    #[test]
    fn epson_document_request_selects_the_feeder_not_the_document_table() {
        let epson = parse_scanimage_help(
            "    --source Document Table|Automatic Document Feeder [Document Table]\n",
        );
        let request = ScanRequest {
            mode: ScanMode::Document,
            duplex: false,
            ..ScanRequest::default()
        };
        let args = command_args(&request, Some(&epson));
        assert!(
            args.contains(&"--source=Automatic Document Feeder".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn epson_duplex_request_selects_the_feeder_not_the_document_table() {
        let epson = parse_scanimage_help(
            "    --source Document Table|Automatic Document Feeder [Document Table]\n    --duplex[=(yes|no)] [no]\n",
        );
        let request = ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            ..ScanRequest::default()
        };
        let args = command_args(&request, Some(&epson));
        assert!(
            args.contains(&"--source=Automatic Document Feeder".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn source_is_set_before_resolution_and_mode() {
        let capabilities = parse_scanimage_help(
            "    --source ADF Front|ADF Duplex [ADF Front]\n    --mode Lineart|Gray|Color [Color]\n    --resolution 75|150|300dpi [150]\n",
        );
        let args = command_args(&ScanRequest::default(), Some(&capabilities));
        let position = |prefix: &str| args.iter().position(|arg| arg.starts_with(prefix));
        let source = position("--source=").unwrap();
        assert!(source < position("--resolution").unwrap(), "{args:?}");
        assert!(source < position("--mode=").unwrap(), "{args:?}");
    }

    #[test]
    fn y_resolution_alone_is_sent_next_to_the_plain_resolution() {
        let request = ScanRequest {
            dpi_x: 200,
            dpi_y: 300,
            ..ScanRequest::default()
        };
        let only_y = parse_scanimage_help(
            "    --resolution 100..600dpi [200]\n    --y-resolution 100..600dpi [200]\n",
        );
        let args = command_args(&request, Some(&only_y));
        assert!(args.windows(2).any(|pair| pair == ["--resolution", "200"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--y-resolution", "300"]));
        assert!(!args.contains(&"--x-resolution".to_string()));

        let both = parse_scanimage_help(
            "    --x-resolution 100..600dpi [200]\n    --y-resolution 100..600dpi [200]\n",
        );
        let args = command_args(&request, Some(&both));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--x-resolution", "200"]));
        assert!(!args.contains(&"--resolution".to_string()));
    }

    #[test]
    fn zero_extent_emits_no_geometry() {
        let request = ScanRequest {
            width: 0,
            height: 0,
            ..ScanRequest::default()
        };
        let args = command_args(&request, None);
        assert!(!args.iter().any(|arg| arg == "-x" || arg == "-y"));
    }

    #[test]
    fn page_size_options_follow_advertised_capabilities() {
        let request = ScanRequest {
            width: 1240,
            height: 0,
            dpi_x: 150,
            dpi_y: 150,
            ..ScanRequest::default()
        };
        let plain = command_args(&request, None);
        assert!(plain.contains(&"-x".to_string()));
        assert!(!plain.iter().any(|arg| arg.starts_with("--page-")));

        let capabilities = parse_scanimage_help(
            "    --page-width 0..215.9mm [210]\n    --page-height 0..355.6mm [297]\n",
        );
        let args = command_args(&request, Some(&capabilities));
        let page = args.iter().position(|arg| arg == "--page-width=209.97");
        let width = args.iter().position(|arg| arg == "-x");
        assert!(page.is_some() && page < width, "page size must precede -x");
        assert!(!args.iter().any(|arg| arg.starts_with("--page-height")));
    }
}
