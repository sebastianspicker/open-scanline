//! SANE backend — empty-safe with scanimage list/open/scan when present.

use crate::backend_process::{
    artifact_quota_for_request, document_batch_timeout, parse_pipe_devices, run_command,
    run_command_with_cancellation, run_contained_command_with_artifact_quota, simulate_backends,
    validate_artifact_quota, ArtifactWatch, CommandSession, TemporaryOutput,
};
pub use crate::backend_process::{
    ArtifactQuota, CommandOutput, CommandRunner, CommandSpec, ImageDecoder, NativeImageDecoder,
};
use crate::core::{
    validate_scan_dpi, ImageBuffer, PixelFormat, Result, ScanError, ScanMode, ScanRequest,
};
use crate::device::{BackendInfo, CancellationToken, DeviceInfo, ScanPagesResult};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
    ) -> Result<CommandOutput> {
        run_command(spec, timeout, cancelled, "scanimage", "SANE scan cancelled")
    }

    fn run_with_cancellation(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&crate::device::CancellationToken>,
    ) -> Result<CommandOutput> {
        run_command_with_cancellation(
            spec,
            timeout,
            cancelled,
            cancellation,
            "scanimage",
            "SANE scan cancelled",
        )
    }

    fn run_with_cancellation_and_artifact_quota(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&crate::device::CancellationToken>,
        artifact_directory: &Path,
        artifact_quota: ArtifactQuota,
    ) -> Result<CommandOutput> {
        run_contained_command_with_artifact_quota(
            spec,
            timeout,
            cancelled,
            cancellation,
            "scanimage",
            "SANE scan cancelled",
            ArtifactWatch {
                directory: artifact_directory,
                quota: artifact_quota,
            },
        )
    }
}

/// True when the `scanimage` acquisition tool appears on PATH.
///
/// `sane-find-scanner` is intentionally insufficient: it can detect a USB
/// device even when no SANE backend is configured and cannot acquire images.
pub fn available() -> bool {
    available_with_cancellation(None)
}

/// Probe `scanimage` while allowing the bounded version command to be cancelled.
pub fn available_with_cancellation(cancellation: Option<&CancellationToken>) -> bool {
    if simulate_backends() {
        return true;
    }
    which("scanimage").is_some_and(|binary| {
        let cancelled = Mutex::new(false);
        SystemCommandRunner
            .run_with_cancellation(
                &CommandSpec {
                    program: binary.display().to_string(),
                    args: vec!["--version".into()],
                },
                Duration::from_secs(2),
                &cancelled,
                cancellation,
            )
            .is_ok_and(|output| output.success)
    })
}

fn which(bin: &str) -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let exe = dir.join(format!("{bin}.exe"));
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }
    }
    None
}

pub fn backend_info() -> BackendInfo {
    backend_info_with_cancellation(None)
}

/// Backend availability with cancellation for the command-backed probe.
pub fn backend_info_with_cancellation(cancellation: Option<&CancellationToken>) -> BackendInfo {
    let avail = available_with_cancellation(cancellation);
    BackendInfo {
        id: "sane".into(),
        name: if avail {
            "SANE scanners (scanimage)".into()
        } else {
            "SANE (unavailable)".into()
        },
        available: avail,
    }
}

fn try_list_scanimage(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    let Some(bin) = which("scanimage") else {
        return Vec::new();
    };
    let cancelled = Mutex::new(false);
    let Ok(output) = SystemCommandRunner.run_with_cancellation(
        &CommandSpec {
            program: bin.display().to_string(),
            args: vec!["-f".into(), "%d|%v %m%n".into()],
        },
        Duration::from_secs(8),
        &cancelled,
        cancellation,
    ) else {
        return Vec::new();
    };
    parse_sane_devices(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_sane_devices(text: &str) -> Vec<DeviceInfo> {
    parse_pipe_devices(text, "sane", "SANE", "sane")
}

/// Device-specific options discovered from `scanimage --help -d <device>`.
/// SANE backends use free-form constraint strings, so the original values are
/// retained and selected by conservative keyword matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaneCapabilities {
    source_values: Vec<String>,
    mode_values: Vec<String>,
    source_option: bool,
    mode_option: bool,
    duplex_option: Option<String>,
    resolution: Option<ResolutionConstraint>,
    x_resolution: Option<ResolutionConstraint>,
    y_resolution: Option<ResolutionConstraint>,
}

impl SaneCapabilities {
    fn is_empty(&self) -> bool {
        !self.source_option
            && !self.mode_option
            && self.duplex_option.is_none()
            && self.resolution.is_none()
            && self.x_resolution.is_none()
            && self.y_resolution.is_none()
    }
}

/// Numeric resolution constraints advertised by a SANE backend.  Backends
/// commonly use either a discrete `75|150|300` set or a `75..1200dpi` range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolutionConstraint {
    values: Vec<u32>,
    range: Option<(u32, u32)>,
}

impl ResolutionConstraint {
    fn closest_to(&self, requested: u32) -> u32 {
        if !self.values.is_empty() {
            return *self
                .values
                .iter()
                .min_by_key(|value| (value.abs_diff(requested), **value))
                .expect("non-empty values are checked above");
        }
        self.range
            .map(|(minimum, maximum)| requested.clamp(minimum, maximum))
            .unwrap_or(requested)
    }
}

/// Parse the stable option names and device-provided values in scanimage help.
/// Unknown lines are ignored so new or vendor-specific options remain safe.
pub fn parse_scanimage_help(text: &str) -> SaneCapabilities {
    let mut capabilities = SaneCapabilities::default();
    for line in text.lines() {
        if option_line(line, "--source") {
            capabilities.source_option = true;
            capabilities.source_values = option_values(line, "--source");
        }
        if option_line(line, "--mode") {
            capabilities.mode_option = true;
            capabilities.mode_values = option_values(line, "--mode");
        }
        for option in ["--duplex", "--adf-duplex", "--source-duplex"] {
            if option_line(line, option) {
                capabilities.duplex_option = Some(option.into());
            }
        }
        if option_line(line, "--resolution") {
            capabilities.resolution = Some(resolution_constraint(line, "--resolution"));
        }
        if option_line(line, "--x-resolution") {
            capabilities.x_resolution = Some(resolution_constraint(line, "--x-resolution"));
        }
        if option_line(line, "--y-resolution") {
            capabilities.y_resolution = Some(resolution_constraint(line, "--y-resolution"));
        }
    }
    capabilities
}

fn option_line(line: &str, option: &str) -> bool {
    option_tail(line, option).is_some()
}

fn option_values(line: &str, option: &str) -> Vec<String> {
    option_constraint(line, option)
        .map(alternative_values)
        .unwrap_or_default()
}

/// Return the text immediately after an exact option spelling.  Matching the
/// spelling rather than using `find` keeps `--source` distinct from options
/// such as `--source-duplex`.
fn option_tail<'a>(line: &'a str, option: &str) -> Option<&'a str> {
    let position = line.find(option)?;
    let before = &line[..position];
    let after = &line[position + option.len()..];
    if !before.chars().last().is_none_or(char::is_whitespace)
        || !after.chars().next().is_none_or(|character| {
            character.is_whitespace() || matches!(character, '=' | '[' | '(' | '{')
        })
    {
        return None;
    }
    Some(after)
}

/// Remove only a terminal `[default]` group.  A leading `[one|two]` group is
/// itself a constraint, so it must remain available to the alternatives parser.
fn option_constraint<'a>(line: &'a str, option: &str) -> Option<&'a str> {
    let mut value = option_tail(line, option)?.trim();
    if let Some(stripped) = value.strip_prefix('=') {
        value = stripped.trim_start();
    }
    let trimmed = value.trim_end();
    if trimmed.ends_with(']') {
        if let Some(start) = trailing_square_group_start(trimmed) {
            if start > 0 {
                value = trimmed[..start].trim_end();
            }
        }
    }
    Some(value.trim())
}

fn trailing_square_group_start(value: &str) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, character) in value.char_indices().rev() {
        match character {
            ']' => depth += 1,
            '[' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract exactly the advertised alternatives.  Placeholder grammar such as
/// `<string>` and trailing defaults are deliberately not values, while vendor
/// alternatives are preserved byte-for-byte apart from surrounding whitespace.
fn alternative_values(constraint: &str) -> Vec<String> {
    let constraint = strip_leading_placeholder(constraint);
    let grouped = grouped_alternatives(constraint).or_else(|| {
        constraint.contains('|').then_some(
            constraint.trim_matches(|character| matches!(character, '(' | ')' | '{' | '}')),
        )
    });
    grouped
        .map(|values| {
            values
                .split('|')
                .map(|value| {
                    value.trim().trim_matches(|character| {
                        matches!(character, '(' | ')' | '{' | '}' | '[' | ']')
                    })
                })
                .filter(|value| valid_option_value(value))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn strip_leading_placeholder(value: &str) -> &str {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            return rest[end + 1..].trim_start();
        }
    }
    value
}

fn grouped_alternatives(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    for (start, opening) in bytes.iter().copied().enumerate() {
        let closing = match opening {
            b'(' => b')',
            b'{' => b'}',
            b'[' => b']',
            _ => continue,
        };
        let mut depth = 0_u32;
        for (offset, character) in bytes[start..].iter().copied().enumerate() {
            if character == opening {
                depth += 1;
            } else if character == closing {
                depth -= 1;
                if depth == 0 {
                    let inner = &value[start + 1..start + offset];
                    if inner.contains('|') {
                        return Some(inner);
                    }
                    break;
                }
            }
        }
    }
    None
}

fn valid_option_value(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['<', '>', '[', ']', '{', '}', '(', ')'])
        && !value.eq_ignore_ascii_case("string")
        && !value.eq_ignore_ascii_case("mode")
}

fn resolution_constraint(line: &str, option: &str) -> ResolutionConstraint {
    let constraint = option_constraint(line, option).unwrap_or_default();
    let values = alternative_values(constraint)
        .iter()
        .filter_map(|value| parse_exact_resolution(value))
        .collect();
    ResolutionConstraint {
        values,
        range: parse_resolution_range(constraint),
    }
}

fn parse_exact_resolution(value: &str) -> Option<u32> {
    let number = value.trim().strip_suffix("dpi").unwrap_or(value.trim());
    number.parse().ok()
}

fn parse_resolution_range(value: &str) -> Option<(u32, u32)> {
    let split = value.find("..")?;
    let before = value[..split]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    let after = value[split + 2..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    let minimum = before.chars().rev().collect::<String>().parse().ok()?;
    let maximum = after.parse().ok()?;
    (minimum <= maximum).then_some((minimum, maximum))
}

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

fn build_scanimage_command(
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
    if request.width == 0 || request.height == 0 {
        return Err(ScanError::Invalid(
            "SANE dimensions must be positive".into(),
        ));
    }
    validate_scan_dpi(request.dpi_x, request.dpi_y)?;
    if let Some(region) = request.region {
        if region.x < 0 || region.y < 0 || region.width == 0 || region.height == 0 {
            return Err(ScanError::Invalid(
                "SANE scan region must have non-negative offsets and positive size".into(),
            ));
        }
    }
    Ok(())
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
    if request.mode == ScanMode::Reflective && capabilities.is_none() {
        return Ok(false);
    }
    let keywords: &[&str] = match (request.mode, request.duplex) {
        (ScanMode::Document, true) => &["duplex", "adf-duplex", "adf duplex"],
        (ScanMode::Reflective, _) => &["flatbed", "platen", "reflective"],
        (ScanMode::Document, false) => &["automatic document feeder", "adf", "feeder", "document"],
        (ScanMode::Film, _) => &["transparency", "tpu", "film", "negative", "slide"],
    };
    let fallback = match request.mode {
        ScanMode::Reflective => "Flatbed",
        ScanMode::Document => "Automatic Document Feeder",
        ScanMode::Film => "Transparency Unit",
    };
    let mut selected = capabilities
        .and_then(|value| matching_value(&value.source_values, keywords))
        .map(str::to_string);
    if selected.is_none() && request.mode == ScanMode::Document && request.duplex {
        selected = capabilities
            .and_then(|value| {
                matching_value(
                    &value.source_values,
                    &["automatic document feeder", "adf", "feeder", "document"],
                )
            })
            .map(str::to_string);
    }
    if let Some(value) = capabilities {
        if value.source_option && value.source_values.is_empty() {
            if request.mode != ScanMode::Reflective {
                return Err(ScanError::Unsupported(format!(
                    "SANE device advertises --source but its constraints cannot select {} scanning",
                    source_name(request.mode)
                )));
            }
            return Ok(false);
        }
        if value.source_option
            && !value.source_values.is_empty()
            && matching_value(&value.source_values, keywords).is_none()
            && !(request.mode == ScanMode::Document
                && request.duplex
                && value.duplex_option.is_some()
                && matching_value(
                    &value.source_values,
                    &["automatic document feeder", "adf", "feeder", "document"],
                )
                .is_some())
        {
            return Err(ScanError::Unsupported(format!(
                "SANE device does not advertise a {} source",
                source_name(request.mode)
            )));
        }
        if !value.source_option && request.mode != ScanMode::Reflective {
            return Err(ScanError::Unsupported(format!(
                "SANE device does not expose a source option for {} scanning",
                source_name(request.mode)
            )));
        }
        if !value.source_option {
            return Ok(false);
        }
    }
    let selected = selected.unwrap_or_else(|| fallback.to_string());
    let source_includes_duplex = selected.to_ascii_lowercase().contains("duplex");
    args.push(format!("--source={selected}"));
    Ok(source_includes_duplex)
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
    let keywords: &[&str] = match request.pixel_format {
        PixelFormat::Gray8 => &["gray", "grey", "grayscale", "greyscale"],
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => &["color", "colour", "rgb", "24 bit", "24bit"],
    };
    let fallback = if request.pixel_format == PixelFormat::Gray8 {
        "Gray"
    } else {
        "Color"
    };
    let selected = capabilities.and_then(|value| matching_value(&value.mode_values, keywords));
    if let Some(value) = capabilities {
        if value.mode_option && value.mode_values.is_empty() {
            if request.pixel_format != PixelFormat::Rgb8 {
                return Err(ScanError::Unsupported(
                    "SANE device advertises --mode but its constraints cannot select the requested pixel format".into(),
                ));
            }
            return Ok(());
        }
        if value.mode_option && selected.is_none() {
            return Err(ScanError::Unsupported(
                "SANE device does not advertise the requested pixel format".into(),
            ));
        }
        if !value.mode_option {
            return Ok(());
        }
    }
    let selected = selected.or(Some(fallback));
    if let Some(selected) = selected {
        args.push(format!("--mode={selected}"));
    }
    Ok(())
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

/// List SANE devices. Empty-safe. Includes `sane:sim` when simulate env set.
pub fn list_devices() -> Vec<DeviceInfo> {
    list_devices_with_cancellation(None)
}

/// List SANE devices while allowing bounded `scanimage` discovery to be cancelled.
pub fn list_devices_with_cancellation(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    let mut devices =
        std::panic::catch_unwind(|| try_list_scanimage(cancellation)).unwrap_or_default();
    if simulate_backends() && !devices.iter().any(|d| d.id == "sane:sim") {
        devices.push(DeviceInfo::new(
            "sane:sim",
            "SANE Simulated Scanner",
            "sane",
        ));
    }
    devices
}

pub fn list_sane_devices_safe() -> Vec<DeviceInfo> {
    list_devices()
}

/// Cancellation-aware variant of [`list_sane_devices_safe`].
pub fn list_sane_devices_safe_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    list_devices_with_cancellation(cancellation)
}

pub struct SaneDeviceSession {
    pub device_id: String,
    /// SANE device name for -d
    sane_name: String,
    simulate: bool,
    session: CommandSession,
    runner: Arc<dyn CommandRunner>,
    decoder: Arc<dyn ImageDecoder>,
    inspect_capabilities: bool,
    capabilities: OnceLock<Option<SaneCapabilities>>,
}

impl SaneDeviceSession {
    pub fn new(device_id: String, sane_name: String, simulate: bool) -> Self {
        Self {
            device_id,
            sane_name,
            simulate,
            session: CommandSession::default(),
            runner: Arc::new(SystemCommandRunner),
            decoder: Arc::new(NativeImageDecoder),
            inspect_capabilities: !simulate,
            capabilities: OnceLock::new(),
        }
    }

    pub fn new_with_adapters(
        device_id: String,
        sane_name: String,
        simulate: bool,
        runner: Arc<dyn CommandRunner>,
        decoder: Arc<dyn ImageDecoder>,
    ) -> Self {
        Self {
            device_id,
            sane_name,
            simulate,
            session: CommandSession::default(),
            runner,
            decoder,
            inspect_capabilities: false,
            capabilities: OnceLock::new(),
        }
    }

    fn capabilities(&self, binary: &Path) -> Option<&SaneCapabilities> {
        self.capabilities
            .get_or_init(|| {
                if !self.inspect_capabilities {
                    return None;
                }
                let spec = CommandSpec {
                    program: binary.display().to_string(),
                    args: vec!["--help".into(), "-d".into(), self.sane_name.clone()],
                };
                let output = self
                    .runner
                    .run_with_cancellation(
                        &spec,
                        Duration::from_secs(8),
                        self.session.cancelled(),
                        self.session.cancellation_token().as_ref(),
                    )
                    .ok()?;
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                let capabilities = parse_scanimage_help(&text);
                (!capabilities.is_empty()).then_some(capabilities)
            })
            .as_ref()
    }

    fn acquire_scanimage(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        let Some(bin) = which("scanimage") else {
            return Err(ScanError::Unsupported(
                "SANE scan requires scanimage CLI".into(),
            ));
        };
        let output_file = TemporaryOutput::new("sane", "png")?;
        let artifact_quota = artifact_quota_for_request(request, 1)?;
        let spec = build_scanimage_command(
            &bin,
            &self.sane_name,
            request,
            output_file.path(),
            self.capabilities(&bin),
        )?;
        let output = self.runner.run_with_cancellation_and_artifact_quota(
            &spec,
            Duration::from_secs(90),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            output_file.directory(),
            artifact_quota,
        )?;
        validate_artifact_quota(output_file.directory(), artifact_quota)?;
        self.session.decode_materialized_output(
            &output,
            output_file.path(),
            self.decoder.as_ref(),
            request,
            "scanimage failed",
        )
    }

    fn acquire_scanimage_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        let Some(binary) = which("scanimage") else {
            return Err(ScanError::Unsupported(
                "SANE scan requires scanimage CLI".into(),
            ));
        };
        let output = TemporaryOutput::new("sane-batch", "png")?;
        let artifact_quota = artifact_quota_for_request(request, max_pages)?;
        let pattern = output.directory().join("page_%06d.png");
        let mut spec = build_scanimage_command(
            &binary,
            &self.sane_name,
            request,
            output.path(),
            self.capabilities(&binary),
        )?;
        spec.args
            .retain(|argument| !argument.starts_with("--output-file="));
        spec.args.extend([
            format!("--batch={}", pattern.display()),
            "--batch-start=1".into(),
            format!("--batch-count={max_pages}"),
        ]);
        let command = self.runner.run_with_cancellation_and_artifact_quota(
            &spec,
            document_batch_timeout(max_pages),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            output.directory(),
            artifact_quota,
        )?;
        validate_artifact_quota(output.directory(), artifact_quota)?;
        let paths = numbered_batch_outputs(output.directory())?;
        let exhausted = feeder_exhausted(&command.stderr);
        if !command.success && !exhausted {
            return Err(ScanError::Unsupported(format!(
                "scanimage batch failed: {}",
                String::from_utf8_lossy(&command.stderr).trim()
            )));
        }
        if paths.len() > max_pages as usize {
            return Err(ScanError::Other(format!(
                "scanimage emitted {} pages beyond the requested limit {max_pages}",
                paths.len()
            )));
        }
        let successful_output = CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let mut emitted = 0_u32;
        for path in paths {
            if self.session.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            let image = self.session.decode_materialized_output(
                &successful_output,
                &path,
                self.decoder.as_ref(),
                request,
                "scanimage batch failed",
            )?;
            emit(image)?;
            emitted += 1;
        }
        if emitted == max_pages {
            Ok(ScanPagesResult::limit_reached(emitted))
        } else {
            Ok(ScanPagesResult::feeder_exhausted(emitted))
        }
    }
}

fn numbered_batch_outputs(directory: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("page_") && name.ends_with(".png"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn feeder_exhausted(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    [
        "out of documents",
        "no documents",
        "document feeder empty",
        "paper empty",
        "no paper",
    ]
    .iter()
    .any(|message| stderr.contains(message))
}

impl crate::device::DeviceSession for SaneDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        crate::device::reject_single_page_duplex(request)?;
        if self.session.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.session.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if self.simulate {
            return crate::device::MockDeviceSession::gradient(request);
        }
        self.acquire_scanimage(request)
    }

    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        crate::device::validate_page_limit(max_pages)?;
        if self.session.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.session.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if request.duplex && request.mode != ScanMode::Document {
            return Err(ScanError::Invalid(
                "duplex is only valid with a document feeder source".into(),
            ));
        }
        if self.simulate {
            for index in 0..max_pages {
                let mut page_request = request.clone();
                page_request.duplex = false;
                page_request.seed = request.seed.saturating_add(index);
                emit(crate::device::MockDeviceSession::gradient(&page_request)?)?;
            }
            return Ok(ScanPagesResult::limit_reached(max_pages));
        }
        if request.mode == ScanMode::Document {
            return self.acquire_scanimage_pages(request, max_pages, emit);
        }
        if request.duplex {
            return Err(ScanError::Unsupported(
                "SANE duplex acquisition requires an ADF source".into(),
            ));
        }
        for index in 0..max_pages {
            let mut page_request = request.clone();
            page_request.seed = request.seed.saturating_add(index);
            emit(self.acquire_scanimage(&page_request)?)?;
        }
        Ok(ScanPagesResult::limit_reached(max_pages))
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn bind_cancellation(&self, token: crate::device::CancellationToken) {
        self.session.bind_cancellation(token);
    }

    fn close(&self) {
        self.session.close();
    }

    fn calibrate(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "backend": "sane",
            "device_id": self.device_id,
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": x.clamp(0.0, 1.0),
            "y_frac": y.clamp(0.0, 1.0),
            "focus": 0.0,
            "backend": "sane",
        })
    }
}

/// Open SANE session for listed device or sim.
pub fn open(device_id: &str) -> Result<SaneDeviceSession> {
    let id = device_id.trim();
    if !id.starts_with("sane:") && id != "sane" {
        return Err(ScanError::DeviceNotFound(format!(
            "not a SANE device id: {id}"
        )));
    }
    if id == "sane:sim" {
        return Ok(SaneDeviceSession::new(id.into(), "sim".into(), true));
    }
    let listed = list_devices();
    if !listed.iter().any(|d| d.id == id) {
        return Err(ScanError::DeviceNotFound(format!(
            "unknown SANE device: {id}"
        )));
    }
    let name = id.strip_prefix("sane:").unwrap_or(id).to_string();
    Ok(SaneDeviceSession::new(id.into(), name, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{PixelFormat, Rect};
    use crate::device::DeviceSession;

    #[test]
    fn list_does_not_panic() {
        let _ = list_devices();
        assert_eq!(backend_info().id, "sane");
    }

    #[test]
    fn sim_open_and_scan() {
        let session = open("sane:sim").expect("open sane:sim");
        let req = ScanRequest {
            width: 10,
            height: 8,
            seed: 3,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        };
        let img = session.scan(&req).expect("scan");
        assert_eq!(img.width, 10);
    }

    #[test]
    fn parses_device_sources_modes_duplex_and_axis_resolutions() {
        let capabilities = parse_scanimage_help(
            r#"
  --source Flatbed|Automatic Document Feeder|ADF Duplex [Flatbed]
  --mode Lineart|Gray|24bit Color [24bit Color]
  --x-resolution 75..1200dpi [300]
  --y-resolution 75..1200dpi [300]
"#,
        );

        assert_eq!(
            capabilities.source_values,
            ["Flatbed", "Automatic Document Feeder", "ADF Duplex"]
        );
        assert_eq!(capabilities.mode_values, ["Lineart", "Gray", "24bit Color"]);
        assert!(capabilities.x_resolution.is_some());
        assert!(capabilities.y_resolution.is_some());
    }

    #[test]
    fn parses_real_style_grouped_alternatives_without_placeholder_fragments() {
        let capabilities = parse_scanimage_help(
            r#"
  --source=[(Flatbed|Automatic Document Feeder|Transparency Unit)] [Flatbed]
  --mode {Color|Gray|Vendor RGB+} [Color]
  --resolution <dpi> (75|150|300|600) [300]
"#,
        );

        assert_eq!(
            capabilities.source_values,
            ["Flatbed", "Automatic Document Feeder", "Transparency Unit"]
        );
        assert_eq!(capabilities.mode_values, ["Color", "Gray", "Vendor RGB+"]);
        assert_eq!(
            capabilities.resolution.as_ref().unwrap().values,
            [75, 150, 300, 600]
        );
        assert!(capabilities
            .source_values
            .iter()
            .chain(&capabilities.mode_values)
            .all(|value| !value.contains(['<', '>', '[', ']', '(', ')', '{', '}'])));
    }

    #[test]
    fn capability_command_maps_adf_duplex_gray_region_and_independent_dpi() {
        let capabilities = parse_scanimage_help(
            r#"
  --source Flatbed|ADF Front|ADF Duplex [Flatbed]
  --mode Gray|Color [Color]
  --x-resolution 75..1200dpi [300]
  --y-resolution 75..1200dpi [300]
"#,
        );
        let request = ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            dpi_x: 300,
            dpi_y: 600,
            width: 600,
            height: 1200,
            region: Some(Rect::new(30, 60, 300, 600)),
            pixel_format: PixelFormat::Gray8,
            ..ScanRequest::default()
        };

        let command = build_scanimage_command(
            Path::new("scanimage"),
            "net:scanner",
            &request,
            Path::new("output.png"),
            Some(&capabilities),
        )
        .unwrap();

        for expected in [
            "--x-resolution",
            "300",
            "--y-resolution",
            "600",
            "--source=ADF Duplex",
            "--mode=Gray",
            "-l",
            "2.54",
            "-t",
            "2.54",
            "-x",
            "25.40",
            "-y",
            "25.40",
        ] {
            assert!(command.args.iter().any(|value| value == expected));
        }
        assert!(!command.args.iter().any(|value| value == "--duplex=yes"));
    }

    #[test]
    fn capability_command_negotiates_discrete_and_range_resolutions() {
        let discrete = parse_scanimage_help(
            "--source Flatbed|ADF [Flatbed]\n--mode Color|Gray [Color]\n--resolution (75|150|300|600) [300]",
        );
        let command = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                dpi_x: 500,
                dpi_y: 500,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
            Some(&discrete),
        )
        .unwrap();
        assert!(command
            .args
            .windows(2)
            .any(|pair| pair == ["--resolution", "600"]));

        let axes = parse_scanimage_help(
            "--source Flatbed|ADF [Flatbed]\n--mode Color|Gray [Color]\n--x-resolution 100..600dpi [300]\n--y-resolution [75|150|300] [150]",
        );
        let command = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                dpi_x: 900,
                dpi_y: 200,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
            Some(&axes),
        )
        .unwrap();
        assert!(command
            .args
            .windows(2)
            .any(|pair| pair == ["--x-resolution", "600"]));
        assert!(command
            .args
            .windows(2)
            .any(|pair| pair == ["--y-resolution", "150"]));
    }

    #[test]
    fn explicit_duplex_option_is_used_with_simplex_adf_source() {
        let capabilities = parse_scanimage_help(
            "--source Flatbed|Automatic Document Feeder [Flatbed]\n--duplex[=(yes|no)] [no]",
        );
        let request = ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            ..ScanRequest::default()
        };
        let command = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &request,
            Path::new("out.png"),
            Some(&capabilities),
        )
        .unwrap();

        assert!(command
            .args
            .iter()
            .any(|value| value == "--source=Automatic Document Feeder"));
        assert!(command.args.iter().any(|value| value == "--duplex=yes"));
    }

    #[test]
    fn unavailable_specialized_source_is_an_explicit_error() {
        let capabilities = parse_scanimage_help("--source Flatbed|ADF [Flatbed]");
        let error = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                mode: ScanMode::Film,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
            Some(&capabilities),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ScanError::Unsupported(message)
                if message.contains("film/transparency")
        ));
    }

    #[test]
    fn unparseable_advertised_source_or_explicit_mode_fails_closed() {
        let source = parse_scanimage_help("--source <string> [Flatbed]");
        let source_error = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                mode: ScanMode::Document,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
            Some(&source),
        )
        .unwrap_err();
        assert!(
            matches!(source_error, ScanError::Unsupported(message) if message.contains("constraints"))
        );

        let mode = parse_scanimage_help("--source Flatbed|ADF [Flatbed]\n--mode <string> [Color]");
        let mode_error = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                pixel_format: PixelFormat::Gray8,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
            Some(&mode),
        )
        .unwrap_err();
        assert!(
            matches!(mode_error, ScanError::Unsupported(message) if message.contains("pixel format"))
        );

        let default_command = build_scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest::default(),
            Path::new("out.png"),
            Some(&parse_scanimage_help(
                "--source <string> [Flatbed]\n--mode <string> [Color]",
            )),
        )
        .unwrap();
        assert!(!default_command
            .args
            .iter()
            .any(|arg| arg.starts_with("--source=")));
        assert!(!default_command
            .args
            .iter()
            .any(|arg| arg.starts_with("--mode=")));
    }

    #[test]
    fn direct_command_builder_preserves_requested_dpi_instead_of_clamping() {
        let command = scanimage_command(
            Path::new("scanimage"),
            "device",
            &ScanRequest {
                dpi_x: 10,
                dpi_y: 10,
                ..ScanRequest::default()
            },
            Path::new("out.png"),
        );
        assert!(command
            .args
            .windows(2)
            .any(|pair| pair == ["--resolution", "10"]));
    }
}
