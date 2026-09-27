use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::{
    DeviceMaintenanceCapabilities, FocusCapability, MaintenanceAvailability,
};
use crate::infrastructure::runtime::CommandSpec;
use std::path::Path;

/// Device-specific options discovered from `scanimage --help -d <device>`.
/// SANE backends use free-form constraint strings, so the original values are
/// retained and selected by conservative keyword matching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaneCapabilities {
    pub(super) source_values: Vec<String>,
    pub(super) mode_values: Vec<String>,
    pub(super) source_option: bool,
    pub(super) mode_option: bool,
    pub(super) duplex_option: Option<String>,
    pub(super) resolution: Option<ResolutionConstraint>,
    pub(super) x_resolution: Option<ResolutionConstraint>,
    pub(super) y_resolution: Option<ResolutionConstraint>,
}

/// Strictly allowlisted maintenance options advertised by `scanimage -A`.
/// No option outside this shape is ever copied from driver output into a
/// maintenance command.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaneMaintenanceOptions {
    pub calibrate: bool,
    pub autofocus: bool,
    pub focus_on_centre: bool,
    pub focus_x_range: Option<(f64, f64)>,
    pub focus_y_range: Option<(f64, f64)>,
}

impl SaneMaintenanceOptions {
    fn focus_capability(&self) -> FocusCapability {
        if self.autofocus {
            if let (Some(x_range), Some(y_range)) = (self.focus_x_range, self.focus_y_range) {
                return FocusCapability::Point {
                    availability: MaintenanceAvailability::Supported,
                    x_range,
                    y_range,
                };
            }
            return FocusCapability::Center {
                availability: MaintenanceAvailability::Supported,
            };
        }
        if self.focus_on_centre {
            return FocusCapability::Center {
                availability: MaintenanceAvailability::Supported,
            };
        }
        FocusCapability::Unsupported {
            reason: "SANE backend did not advertise autofocus".into(),
        }
    }

    pub(super) fn capabilities(&self) -> DeviceMaintenanceCapabilities {
        DeviceMaintenanceCapabilities {
            calibration: if self.calibrate {
                MaintenanceAvailability::Supported
            } else {
                MaintenanceAvailability::Unsupported {
                    reason: "SANE backend did not advertise --calibrate".into(),
                }
            },
            focus: self.focus_capability(),
        }
    }
}

/// The only maintenance actions Open Scanline constructs for SANE.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaneMaintenanceAction {
    Calibrate,
    FocusCenter,
    FocusPoint { x_fraction: f64, y_fraction: f64 },
}

impl SaneCapabilities {
    pub(super) fn is_empty(&self) -> bool {
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
pub(super) struct ResolutionConstraint {
    values: Vec<u32>,
    range: Option<(u32, u32)>,
}

impl ResolutionConstraint {
    pub(super) fn closest_to(&self, requested: u32) -> u32 {
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
        parse_scanimage_option_line(&mut capabilities, line);
    }
    capabilities
}

fn parse_scanimage_option_line(capabilities: &mut SaneCapabilities, line: &str) {
    parse_source_option(capabilities, line);
    parse_mode_option(capabilities, line);
    parse_duplex_option(capabilities, line);
    parse_resolution_options(capabilities, line);
}

fn parse_source_option(capabilities: &mut SaneCapabilities, line: &str) {
    if option_line(line, "--source") {
        capabilities.source_option = true;
        capabilities.source_values = option_values(line, "--source");
    }
}

fn parse_mode_option(capabilities: &mut SaneCapabilities, line: &str) {
    if option_line(line, "--mode") {
        capabilities.mode_option = true;
        capabilities.mode_values = option_values(line, "--mode");
    }
}

fn parse_duplex_option(capabilities: &mut SaneCapabilities, line: &str) {
    for option in ["--duplex", "--adf-duplex", "--source-duplex"] {
        if option_line(line, option) {
            capabilities.duplex_option = Some(option.into());
        }
    }
}

fn parse_resolution_options(capabilities: &mut SaneCapabilities, line: &str) {
    assign_resolution(line, "--resolution", &mut capabilities.resolution);
    assign_resolution(line, "--x-resolution", &mut capabilities.x_resolution);
    assign_resolution(line, "--y-resolution", &mut capabilities.y_resolution);
}

fn assign_resolution(line: &str, option: &str, target: &mut Option<ResolutionConstraint>) {
    if option_line(line, option) {
        *target = Some(resolution_constraint(line, option));
    }
}

/// Parse only maintenance options with fixed spellings from `scanimage -A`.
/// Hostile or malformed range text is ignored rather than treated as a
/// command fragment or a point-focus capability.
pub fn parse_scanimage_maintenance_options(text: &str) -> SaneMaintenanceOptions {
    let mut options = SaneMaintenanceOptions::default();
    for line in text.lines() {
        options.calibrate |= zero_arg_option_line(line, "--calibrate");
        options.autofocus |= zero_arg_option_line(line, "--autofocus");
        options.focus_on_centre |= zero_arg_option_line(line, "--focus-on-centre");
        if options.focus_x_range.is_none() {
            options.focus_x_range = focus_range(line, "--focusx");
        }
        if options.focus_y_range.is_none() {
            options.focus_y_range = focus_range(line, "--focusy");
        }
    }
    options
}

fn zero_arg_option_line(line: &str, option: &str) -> bool {
    maintenance_option_tail(line, option).is_some_and(|tail| tail.trim().is_empty())
}

fn focus_range(line: &str, option: &str) -> Option<(f64, f64)> {
    let tail = maintenance_option_tail(line, option)?.trim_start();
    let tail = tail.strip_prefix('=').unwrap_or(tail).trim_start();
    let candidate = tail
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(range, _)| range))
        .unwrap_or_else(|| tail.split_whitespace().next().unwrap_or_default());
    let (minimum, maximum) = candidate.split_once("..")?;
    let minimum = minimum.trim().parse::<f64>().ok()?;
    let maximum = maximum.trim().parse::<f64>().ok()?;
    (minimum.is_finite() && maximum.is_finite() && minimum <= maximum).then_some((minimum, maximum))
}

/// Maintenance discovery accepts only an option declaration at the start of
/// the trimmed line. This prevents descriptive prose from advertising an
/// operation merely because it mentions an allowlisted spelling.
fn maintenance_option_tail<'a>(line: &'a str, option: &str) -> Option<&'a str> {
    let after = line.trim_start().strip_prefix(option)?;
    if !after.chars().next().is_none_or(|character| {
        character.is_whitespace() || matches!(character, '=' | '[' | '(' | '{')
    }) {
        return None;
    }
    Some(after)
}

/// Construct the bounded SANE option-inspection command. Device names are an
/// argument of `-d`, never interpolated into a shell string.
pub fn scanimage_maintenance_inspection_command(binary: &Path, sane_name: &str) -> CommandSpec {
    CommandSpec {
        program: binary.display().to_string(),
        args: vec!["-d".into(), sane_name.into(), "-A".into()],
    }
}

/// Construct one allowlisted, no-scan maintenance action from an inspected
/// option set. Point focus is rejected when either coordinate range is absent.
pub fn scanimage_maintenance_command(
    binary: &Path,
    sane_name: &str,
    options: &SaneMaintenanceOptions,
    action: SaneMaintenanceAction,
) -> Result<CommandSpec> {
    let mut args = vec!["-d".into(), sane_name.into()];
    args.extend(maintenance_action_args(options, action)?);
    args.push("--dont-scan".into());
    Ok(CommandSpec {
        program: binary.display().to_string(),
        args,
    })
}

fn maintenance_action_args(
    options: &SaneMaintenanceOptions,
    action: SaneMaintenanceAction,
) -> Result<Vec<String>> {
    match action {
        SaneMaintenanceAction::Calibrate => calibration_args(options),
        SaneMaintenanceAction::FocusCenter => center_focus_args(options),
        SaneMaintenanceAction::FocusPoint {
            x_fraction,
            y_fraction,
        } => point_focus_args(options, x_fraction, y_fraction),
    }
}

fn calibration_args(options: &SaneMaintenanceOptions) -> Result<Vec<String>> {
    if options.calibrate {
        return Ok(vec!["--calibrate".into()]);
    }
    Err(ScanError::Unsupported(
        "SANE backend did not advertise --calibrate".into(),
    ))
}

fn center_focus_args(options: &SaneMaintenanceOptions) -> Result<Vec<String>> {
    if options.autofocus {
        return Ok(vec!["--autofocus".into()]);
    }
    if options.focus_on_centre {
        return Ok(vec!["--focus-on-centre".into()]);
    }
    Err(unsupported_focus())
}

fn point_focus_args(
    options: &SaneMaintenanceOptions,
    x_fraction: f64,
    y_fraction: f64,
) -> Result<Vec<String>> {
    if !options.autofocus {
        return Err(unsupported_focus());
    }
    validate_focus_fraction(x_fraction, y_fraction)?;
    let x = mapped_focus_coordinate(x_fraction, options.focus_x_range, "--focusx")?;
    let y = mapped_focus_coordinate(y_fraction, options.focus_y_range, "--focusy")?;
    Ok(vec![
        "--focusx".into(),
        x.to_string(),
        "--focusy".into(),
        y.to_string(),
        "--autofocus".into(),
    ])
}

fn validate_focus_fraction(x_fraction: f64, y_fraction: f64) -> Result<()> {
    if x_fraction.is_finite()
        && y_fraction.is_finite()
        && (0.0..=1.0).contains(&x_fraction)
        && (0.0..=1.0).contains(&y_fraction)
    {
        return Ok(());
    }
    Err(ScanError::Invalid(
        "focus coordinates must be finite normalized values between 0 and 1".into(),
    ))
}

fn mapped_focus_coordinate(fraction: f64, range: Option<(f64, f64)>, option: &str) -> Result<f64> {
    let (minimum, maximum) = range.ok_or_else(|| {
        ScanError::Unsupported(format!(
            "SANE point focus requires an advertised {option} range"
        ))
    })?;
    map_focus_coordinate(fraction, minimum, maximum)
}

fn unsupported_focus() -> ScanError {
    ScanError::Unsupported("SANE backend did not advertise the requested focus operation".into())
}

fn map_focus_coordinate(fraction: f64, minimum: f64, maximum: f64) -> Result<f64> {
    let value = (1.0 - fraction) * minimum + fraction * maximum;
    if !value.is_finite() {
        return Err(ScanError::Invalid(
            "mapped SANE focus coordinate is not finite".into(),
        ));
    }
    Ok(value)
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
        let Some(end) = matching_group_end(bytes, start, opening) else {
            continue;
        };
        let inner = &value[start + 1..end];
        if inner.contains('|') {
            return Some(inner);
        }
    }
    None
}

fn matching_group_end(bytes: &[u8], start: usize, opening: u8) -> Option<usize> {
    let closing = match opening {
        b'(' => b')',
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0_u32;
    for (offset, character) in bytes[start..].iter().copied().enumerate() {
        match character {
            value if value == opening => depth += 1,
            value if value == closing => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
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
