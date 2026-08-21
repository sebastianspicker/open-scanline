//! Windows WIA backend — list/open/scan empty-safe; real COM transfer when device present.

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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Small process seam for deterministic command construction, cancellation, and
/// platform-independent tests. Production uses [`SystemCommandRunner`].
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
    ) -> Result<CommandOutput> {
        run_command(
            spec,
            timeout,
            cancelled,
            "WIA command",
            "WIA scan cancelled",
        )
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
            "WIA command",
            "WIA scan cancelled",
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
            "WIA command",
            "WIA scan cancelled",
            ArtifactWatch {
                directory: artifact_directory,
                quota: artifact_quota,
            },
        )
    }
}

fn powershell_spec(script: String) -> CommandSpec {
    CommandSpec {
        program: POWERSHELL.into(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            script,
        ],
    }
}

const POWERSHELL: &str = "powershell";

/// Check the Windows PowerShell executable rather than reporting an unusable
/// backend when PowerShell has been removed from PATH.
fn powershell_available() -> bool {
    powershell_available_with_cancellation(None)
}

fn powershell_available_with_cancellation(_cancellation: Option<&CancellationToken>) -> bool {
    #[cfg(target_os = "windows")]
    {
        let cancelled = Mutex::new(false);
        SystemCommandRunner
            .run_with_cancellation(
                &CommandSpec {
                    program: POWERSHELL.into(),
                    args: vec![
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-Command".into(),
                        "$null".into(),
                    ],
                },
                Duration::from_secs(2),
                &cancelled,
                _cancellation,
            )
            .is_ok_and(|output| output.success)
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Platform-appropriate availability: Windows with a usable PowerShell
/// command. Simulation remains independently available through `list_devices`.
pub fn available() -> bool {
    cfg!(target_os = "windows") && powershell_available()
}

/// Availability with cancellation for the bounded PowerShell probe.
pub fn available_with_cancellation(cancellation: Option<&CancellationToken>) -> bool {
    cfg!(target_os = "windows") && powershell_available_with_cancellation(cancellation)
}

pub fn backend_info() -> BackendInfo {
    backend_info_with_cancellation(None)
}

/// Backend info with cancellation for the command-backed availability probe.
pub fn backend_info_with_cancellation(cancellation: Option<&CancellationToken>) -> BackendInfo {
    let available = available_with_cancellation(cancellation);
    BackendInfo {
        id: "wia".into(),
        name: if available {
            "Windows WIA Scanners".into()
        } else {
            "WIA (unavailable on this OS)".into()
        },
        available,
    }
}

fn wia_enumeration_script() -> &'static str {
    r#"
$ErrorActionPreference = 'Stop'
try {
  $dm = New-Object -ComObject WIA.DeviceManager
  $names = @()
  foreach ($d in $dm.DeviceInfos) {
    # WiaDeviceType.Scanner is 1; cameras and video devices are not scanners.
    try { $type = [int]$d.Type } catch { $type = [int]$d.Properties('Type').Value }
    if ($type -ne 1) { continue }
    $n = $d.Properties('Name').Value
    $id = $d.DeviceID
    if (-not $n) { $n = $id }
    $names += ($id + '|' + $n)
  }
  $names -join "`n"
} catch {
  ''
}
"#
}

/// Attempt WIA enumeration via PowerShell WIA.DeviceManager (empty-safe).
fn try_list_via_powershell(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    if !available_with_cancellation(cancellation) {
        return Vec::new();
    }
    let cancelled = Mutex::new(false);
    let Ok(output) = SystemCommandRunner.run_with_cancellation(
        &powershell_spec(wia_enumeration_script().into()),
        Duration::from_secs(8),
        &cancelled,
        cancellation,
    ) else {
        return Vec::new();
    };
    parse_wia_devices(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_wia_devices(text: &str) -> Vec<DeviceInfo> {
    parse_pipe_devices(text, "wia", "WIA", "wia")
}

/// Build the WIA COM transfer invocation as data for inspection/injection.
/// Values are quoted for the PowerShell script; dimensions and DPI remain
/// numeric data. Callers validate request bounds before invoking the adapter.
pub fn wia_transfer_command(
    raw_id: &str,
    request: &ScanRequest,
    output_path: &Path,
) -> CommandSpec {
    let out_ps = output_path.display().to_string().replace('\'', "''");
    let raw_ps = raw_id.replace('\'', "''");
    let (x, y, width, height) = request
        .region
        .map(|region| {
            (
                region.x.max(0),
                region.y.max(0),
                region.width.max(1),
                region.height.max(1),
            )
        })
        .unwrap_or((0, 0, request.width.max(1), request.height.max(1)));
    let dpi_x = request.dpi_x;
    let dpi_y = request.dpi_y;
    let intent = match request.pixel_format {
        PixelFormat::Gray8 => 2, // WIA_INTENT_IMAGE_TYPE_GRAYSCALE
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => 1, // WIA_INTENT_IMAGE_TYPE_COLOR
    };
    let source = match request.mode {
        ScanMode::Reflective => "reflective",
        ScanMode::Document => "document",
        ScanMode::Film => "film",
    };
    let duplex = if request.duplex { "$true" } else { "$false" };
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$out = '{out_ps}'
$want = '{raw_ps}'
$wantMode = '{source}'
$wantDuplex = {duplex}

# WIA 2.0 item category property and documented category GUIDs.
$WiaIpaItemCategory = 4125
$WiaCategoryFlatbed = 'fb607b1f-43f3-488b-855b-fb703ec342a6'
$WiaCategoryFeeder = 'fe131934-f84c-42ad-8da4-6129cddd7288'
$WiaCategoryFeederFront = '4823175c-3b28-487b-a7e6-eebc17614fd1'
$WiaCategoryFeederBack = '61ca74d4-39db-42aa-89b1-8c19c9cd4c23'
$WiaCategoryFilm = 'fcf65be7-3ce3-4473-af85-f5d37d21b68a'

# WIA_DPS_DOCUMENT_HANDLING_CAPABILITIES / _SELECT and their flag values.
$WiaDpsDocumentHandlingCapabilities = 3086
$WiaDpsDocumentHandlingSelect = 3088
$WiaDocumentHandlingFeeder = 1
$WiaDocumentHandlingDuplex = 4
$WiaDocumentHandlingFrontOnly = 32

function Get-WiaProperty($owner, [int]$id) {{
  try {{ return $owner.Properties.Item($id) }} catch {{ return $null }}
}}

function Get-WiaItemCategory($candidate) {{
  $property = Get-WiaProperty $candidate $WiaIpaItemCategory
  if ($null -eq $property) {{ return '' }}
  return ([string]$property.Value).Trim('{{}}').ToLowerInvariant()
}}

function Get-WiaItemText($candidate) {{
  $parts = @([string]$candidate.Name)
  foreach ($propertyId in @(4098, 4099)) {{
    $property = Get-WiaProperty $candidate $propertyId
    if ($null -ne $property) {{ $parts += [string]$property.Value }}
  }}
  return ($parts -join ' ').ToLowerInvariant()
}}

function Find-WiaItem($items, [string[]]$categories, [string[]]$names) {{
  foreach ($candidate in $items) {{
    if ($categories -contains (Get-WiaItemCategory $candidate)) {{ return $candidate }}
  }}
  foreach ($candidate in $items) {{
    $text = Get-WiaItemText $candidate
    foreach ($name in $names) {{
      if ($text.Contains($name)) {{ return $candidate }}
    }}
  }}
  return $null
}}

$dm = New-Object -ComObject WIA.DeviceManager
$dev = $null
foreach ($info in $dm.DeviceInfos) {{
  if ($info.DeviceID -eq $want) {{ $dev = $info.Connect(); break }}
}}
if ($null -eq $dev) {{ throw 'device not connected' }}

$items = @()
for ($index = 1; $index -le $dev.Items.Count; $index++) {{ $items += $dev.Items.Item($index) }}
if ($wantMode -eq 'document') {{
  $item = Find-WiaItem $items @($WiaCategoryFeeder, $WiaCategoryFeederFront, $WiaCategoryFeederBack) @('automatic document feeder', 'adf', 'feeder', 'document')
  if ($null -eq $item) {{ throw 'WIA device does not expose a document feeder item' }}
}} elseif ($wantMode -eq 'film') {{
  $item = Find-WiaItem $items @($WiaCategoryFilm) @('transparency', 'tpu', 'film', 'negative', 'slide')
  if ($null -eq $item) {{ throw 'WIA device does not expose a film item' }}
}} else {{
  $item = Find-WiaItem $items @($WiaCategoryFlatbed) @('flatbed', 'platen', 'reflective')
  if ($null -eq $item) {{
    if ($items.Count -eq 1) {{
      # A single unknown item is the only conservative reflective fallback.
      $item = $items[0]
    }} else {{
      throw 'WIA device does not expose a reflective item'
    }}
  }}
}}

if ($wantDuplex -and $wantMode -ne 'document') {{ throw 'duplex is only valid with a document feeder source' }}
if ($wantMode -eq 'document') {{
  $selectProperty = Get-WiaProperty $dev $WiaDpsDocumentHandlingSelect
  if ($null -eq $selectProperty) {{ $selectProperty = Get-WiaProperty $item $WiaDpsDocumentHandlingSelect }}
  if ($null -ne $selectProperty) {{
    $documentHandling = ([int]$selectProperty.Value -bor $WiaDocumentHandlingFeeder)
    if ($wantDuplex) {{
      $capabilities = Get-WiaProperty $dev $WiaDpsDocumentHandlingCapabilities
      if ($null -eq $capabilities) {{ $capabilities = Get-WiaProperty $item $WiaDpsDocumentHandlingCapabilities }}
      if ($null -eq $capabilities -or (([int]$capabilities.Value -band $WiaDocumentHandlingDuplex) -eq 0)) {{
        throw 'WIA device does not advertise duplex acquisition'
      }}
      $documentHandling = (($documentHandling -bor $WiaDocumentHandlingDuplex) -band (-bnot $WiaDocumentHandlingFrontOnly))
    }}
    $selectProperty.Value = $documentHandling
  }} elseif ($wantDuplex) {{
    throw 'WIA device does not expose document handling controls for duplex acquisition'
  }}
}}

# WIA_IPS_CUR_INTENT, XRES, YRES, XPOS, YPOS, XEXTENT, and YEXTENT.
$item.Properties.Item(6146).Value = {intent}
$item.Properties.Item(6147).Value = {dpi_x}
$item.Properties.Item(6148).Value = {dpi_y}
$item.Properties.Item(6149).Value = {x}
$item.Properties.Item(6150).Value = {y}
$item.Properties.Item(6151).Value = {width}
$item.Properties.Item(6152).Value = {height}
$img = $item.Transfer()
$img.SaveFile($out)
"#
    );
    powershell_spec(script)
}

/// Build one WIA feeder transfer process which keeps one COM device connection
/// alive while it materializes at most `max_pages` scanned sides.  The output
/// names are deliberately zero-padded so the Rust side can decode them in a
/// stable, numeric order independent of filesystem enumeration order.
fn wia_transfer_pages_command(
    raw_id: &str,
    request: &ScanRequest,
    output_directory: &Path,
    max_pages: u32,
) -> CommandSpec {
    let output_ps = output_directory.display().to_string().replace('\'', "''");
    let raw_ps = raw_id.replace('\'', "''");
    let (x, y, width, height) = request
        .region
        .map(|region| {
            (
                region.x.max(0),
                region.y.max(0),
                region.width.max(1),
                region.height.max(1),
            )
        })
        .unwrap_or((0, 0, request.width.max(1), request.height.max(1)));
    let dpi_x = request.dpi_x;
    let dpi_y = request.dpi_y;
    let intent = match request.pixel_format {
        PixelFormat::Gray8 => 2,
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => 1,
    };
    let duplex = if request.duplex { "$true" } else { "$false" };
    let script = format!(
        r#"
$ErrorActionPreference = 'Stop'
$outDir = '{output_ps}'
$want = '{raw_ps}'
$wantDuplex = {duplex}
$wantSides = {max_pages}

$WiaIpaItemCategory = 4125
$WiaCategoryFeeder = 'fe131934-f84c-42ad-8da4-6129cddd7288'
$WiaCategoryFeederFront = '4823175c-3b28-487b-a7e6-eebc17614fd1'
$WiaCategoryFeederBack = '61ca74d4-39db-42aa-89b1-8c19c9cd4c23'
$WiaDpsDocumentHandlingCapabilities = 3086
$WiaDpsDocumentHandlingSelect = 3088
$WiaDpsPages = 3096
$WiaDocumentHandlingFeeder = 1
$WiaDocumentHandlingDuplex = 4
$WiaDocumentHandlingFrontFirst = 8
$WiaDocumentHandlingFrontOnly = 32

function Get-WiaProperty($owner, [int]$id) {{
  try {{ return $owner.Properties.Item($id) }} catch {{ return $null }}
}}

function Get-WiaItemCategory($candidate) {{
  $property = Get-WiaProperty $candidate $WiaIpaItemCategory
  if ($null -eq $property) {{ return '' }}
  return ([string]$property.Value).Trim('{{}}').ToLowerInvariant()
}}

function Find-WiaFeeder($items) {{
  foreach ($candidate in $items) {{
    if (@($WiaCategoryFeeder, $WiaCategoryFeederFront, $WiaCategoryFeederBack) -contains (Get-WiaItemCategory $candidate)) {{ return $candidate }}
  }}
  foreach ($candidate in $items) {{
    $description = Get-WiaProperty $candidate 4098
    $descriptionText = if ($null -eq $description) {{ '' }} else {{ [string]$description.Value }}
    $text = (([string]$candidate.Name) + ' ' + $descriptionText).ToLowerInvariant()
    if ($text.Contains('feeder') -or $text.Contains('adf') -or $text.Contains('document')) {{ return $candidate }}
  }}
  return $null
}}

function Test-WiaFeederExhausted($errorRecord) {{
  $text = [string]$errorRecord
  return $text -match '(?i)(paper|document|feeder).{{0,40}}(empty|emptying|out|none|no more)|no.{{0,40}}(paper|document|feeder)|wia_error_paper_empty'
}}

$dm = New-Object -ComObject WIA.DeviceManager
$dev = $null
foreach ($info in $dm.DeviceInfos) {{
  if ($info.DeviceID -eq $want) {{ $dev = $info.Connect(); break }}
}}
if ($null -eq $dev) {{ throw 'device not connected' }}

$items = @()
for ($index = 1; $index -le $dev.Items.Count; $index++) {{ $items += $dev.Items.Item($index) }}
$item = Find-WiaFeeder $items
if ($null -eq $item) {{ throw 'WIA device does not expose a document feeder item' }}

$selectProperty = Get-WiaProperty $dev $WiaDpsDocumentHandlingSelect
if ($null -eq $selectProperty) {{ $selectProperty = Get-WiaProperty $item $WiaDpsDocumentHandlingSelect }}
if ($null -eq $selectProperty) {{ throw 'WIA device does not expose document handling controls' }}
$capabilities = Get-WiaProperty $dev $WiaDpsDocumentHandlingCapabilities
if ($null -eq $capabilities) {{ $capabilities = Get-WiaProperty $item $WiaDpsDocumentHandlingCapabilities }}
$documentHandling = ([int]$selectProperty.Value -bor $WiaDocumentHandlingFeeder)
if ($null -ne $capabilities -and (([int]$capabilities.Value -band $WiaDocumentHandlingFrontFirst) -ne 0)) {{
  $documentHandling = ($documentHandling -bor $WiaDocumentHandlingFrontFirst)
}}
if ($wantDuplex) {{
  if ($null -eq $capabilities -or (([int]$capabilities.Value -band $WiaDocumentHandlingDuplex) -eq 0)) {{
    throw 'WIA device does not advertise duplex acquisition'
  }}
  $documentHandling = (($documentHandling -bor $WiaDocumentHandlingDuplex) -band (-bnot $WiaDocumentHandlingFrontOnly))
}}
$selectProperty.Value = $documentHandling

$pagesProperty = Get-WiaProperty $dev $WiaDpsPages
if ($null -eq $pagesProperty) {{ $pagesProperty = Get-WiaProperty $item $WiaDpsPages }}
if ($null -ne $pagesProperty) {{ $pagesProperty.Value = $wantSides }}

$item.Properties.Item(6146).Value = {intent}
$item.Properties.Item(6147).Value = {dpi_x}
$item.Properties.Item(6148).Value = {dpi_y}
$item.Properties.Item(6149).Value = {x}
$item.Properties.Item(6150).Value = {y}
$item.Properties.Item(6151).Value = {width}
$item.Properties.Item(6152).Value = {height}

for ($side = 1; $side -le $wantSides; $side++) {{
  try {{
    $path = Join-Path $outDir ('page_{{0:D6}}.png' -f $side)
    $img = $item.Transfer()
    $img.SaveFile($path)
  }} catch {{
    if (Test-WiaFeederExhausted $_) {{ Write-Output 'OSL_FEEDER_EXHAUSTED'; break }}
    throw
  }}
}}
"#
    );
    powershell_spec(script)
}

fn numbered_page_outputs(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut pages = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.strip_prefix("page_")
                        .and_then(|number| number.strip_suffix(".png"))
                        .is_some_and(|number| {
                            number.len() == 6 && number.bytes().all(|b| b.is_ascii_digit())
                        })
                })
        })
        .collect::<Vec<_>>();
    pages.sort_by_key(|path| {
        path.file_stem()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("page_"))
            .and_then(|number| number.parse::<u32>().ok())
            .unwrap_or(u32::MAX)
    });
    Ok(pages)
}

fn feeder_exhausted(output: &CommandOutput) -> bool {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    text.contains("osl_feeder_exhausted")
        || [
            "out of documents",
            "no documents",
            "document feeder empty",
            "paper empty",
            "no paper",
        ]
        .iter()
        .any(|message| text.contains(message))
}

/// List WIA devices. Empty-safe; cached. Includes `wia:sim` when simulate env set.
pub fn list_devices() -> Vec<DeviceInfo> {
    list_devices_with_cancellation(None)
}

/// List WIA devices while allowing bounded PowerShell discovery to be cancelled.
pub fn list_devices_with_cancellation(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    static CACHE: OnceLock<Vec<DeviceInfo>> = OnceLock::new();
    if cancellation.is_some() {
        return list_devices_uncached(cancellation);
    }
    let mut devices = CACHE
        .get_or_init(|| {
            if std::env::var("OPEN_SCANLINE_SKIP_WIA").ok().as_deref() == Some("1") {
                return Vec::new();
            }
            std::panic::catch_unwind(|| try_list_via_powershell(None)).unwrap_or_default()
        })
        .clone();
    if simulate_backends() && !devices.iter().any(|d| d.id == "wia:sim") {
        devices.push(DeviceInfo::new("wia:sim", "WIA Simulated Scanner", "wia"));
    }
    devices
}

fn list_devices_uncached(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    if std::env::var("OPEN_SCANLINE_SKIP_WIA").ok().as_deref() == Some("1") {
        return Vec::new();
    }
    let mut devices =
        std::panic::catch_unwind(|| try_list_via_powershell(cancellation)).unwrap_or_default();
    if simulate_backends() && !devices.iter().any(|device| device.id == "wia:sim") {
        devices.push(DeviceInfo::new("wia:sim", "WIA Simulated Scanner", "wia"));
    }
    devices
}

pub fn list_wia_devices_safe() -> Vec<DeviceInfo> {
    list_devices()
}

/// Cancellation-aware variant of [`list_wia_devices_safe`].
pub fn list_wia_devices_safe_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    list_devices_with_cancellation(cancellation)
}

/// Full WIA session: open succeeds for listed devices; scan attempts transfer.
pub struct WiaDeviceSession {
    pub device_id: String,
    /// Raw WIA DeviceID (without wia: prefix)
    raw_id: String,
    simulate: bool,
    session: CommandSession,
    cal: Mutex<Option<CalTables>>,
    focus: Mutex<Option<(f64, f64)>>,
    runner: Arc<dyn CommandRunner>,
    decoder: Arc<dyn ImageDecoder>,
}

#[derive(Clone)]
struct CalTables {
    dark: Vec<i32>,
    flat: Vec<f64>,
}

impl WiaDeviceSession {
    pub fn new(device_id: String, raw_id: String, simulate: bool) -> Self {
        Self {
            device_id,
            raw_id,
            simulate,
            session: CommandSession::default(),
            cal: Mutex::new(None),
            focus: Mutex::new(None),
            runner: Arc::new(SystemCommandRunner),
            decoder: Arc::new(NativeImageDecoder),
        }
    }

    pub fn new_with_adapters(
        device_id: String,
        raw_id: String,
        simulate: bool,
        runner: Arc<dyn CommandRunner>,
        decoder: Arc<dyn ImageDecoder>,
    ) -> Self {
        Self {
            device_id,
            raw_id,
            simulate,
            session: CommandSession::default(),
            cal: Mutex::new(None),
            focus: Mutex::new(None),
            runner,
            decoder,
        }
    }

    /// Acquire via PowerShell COM Transfer → temp PNG → load_image.
    fn acquire_com(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let output_file = TemporaryOutput::new("wia", "png")?;
        let artifact_quota = artifact_quota_for_request(request, 1)?;
        let spec = wia_transfer_command(&self.raw_id, request, output_file.path());
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
            "WIA acquire failed",
        )
    }

    fn acquire_com_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let output = TemporaryOutput::new("wia-pages", "png")?;
        let artifact_quota = artifact_quota_for_request(request, max_pages)?;
        let spec = wia_transfer_pages_command(&self.raw_id, request, output.directory(), max_pages);
        let command_output = self.runner.run_with_cancellation_and_artifact_quota(
            &spec,
            document_batch_timeout(max_pages),
            self.session.cancelled(),
            self.session.cancellation_token().as_ref(),
            output.directory(),
            artifact_quota,
        )?;
        validate_artifact_quota(output.directory(), artifact_quota)?;
        if self.session.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let exhausted = feeder_exhausted(&command_output);
        if !command_output.success && !exhausted {
            return Err(ScanError::Unsupported(format!(
                "WIA batch acquire failed: {}",
                String::from_utf8_lossy(&command_output.stderr).trim()
            )));
        }
        let paths = numbered_page_outputs(output.directory())?;
        if paths.len() > max_pages as usize {
            return Err(ScanError::Other(format!(
                "WIA emitted {} pages beyond the requested limit {max_pages}",
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
            let mut image = self.session.decode_materialized_output(
                &successful_output,
                &path,
                self.decoder.as_ref(),
                request,
                "WIA batch acquire failed",
            )?;
            if let Some(cal) = self.cal.lock().ok().and_then(|tables| tables.clone()) {
                image = crate::device::apply_flat_dark_cal(&image, &cal.dark, &cal.flat)?;
            }
            emit(image)?;
            emitted += 1;
        }
        if emitted == max_pages {
            Ok(ScanPagesResult::limit_reached(emitted))
        } else {
            Ok(ScanPagesResult::feeder_exhausted(emitted))
        }
    }

    fn sim_gradient(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        crate::device::MockDeviceSession::gradient(request)
    }
}

impl crate::device::DeviceSession for WiaDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        crate::device::reject_single_page_duplex(request)?;
        if self.session.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.session.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let mut img = if self.simulate {
            self.sim_gradient(request)?
        } else {
            self.acquire_com(request)?
        };
        if let Some(cal) = self.cal.lock().ok().and_then(|g| g.clone()) {
            img = crate::device::apply_flat_dark_cal(&img, &cal.dark, &cal.flat)?;
        }
        Ok(img)
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
                if self.session.is_cancelled() {
                    return Err(ScanError::Cancelled("scan cancelled".into()));
                }
                let mut page_request = request.clone();
                page_request.duplex = false;
                page_request.seed = request.seed.saturating_add(index);
                let mut image = self.sim_gradient(&page_request)?;
                if let Some(cal) = self.cal.lock().ok().and_then(|tables| tables.clone()) {
                    image = crate::device::apply_flat_dark_cal(&image, &cal.dark, &cal.flat)?;
                }
                emit(image)?;
            }
            return Ok(ScanPagesResult::limit_reached(max_pages));
        }
        if request.mode == ScanMode::Document {
            return self.acquire_com_pages(request, max_pages, emit);
        }
        if request.duplex {
            return Err(ScanError::Unsupported(
                "WIA duplex acquisition requires an ADF source".into(),
            ));
        }
        for index in 0..max_pages {
            let mut page_request = request.clone();
            page_request.seed = request.seed.saturating_add(index);
            emit(self.acquire_com(&page_request)?)?;
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
        use serde_json::json;
        if self.session.is_closed() {
            return json!({"ok": false, "status": "closed", "backend": "wia"});
        }
        // Hardware WIA has no portable calibrate; sim stores tables like mock.
        if self.simulate {
            let (dark, flat) = crate::device::synthetic_cal_tables();
            if let Ok(mut g) = self.cal.lock() {
                *g = Some(CalTables { dark, flat });
            }
            return json!({
                "ok": true,
                "status": "calibrated",
                "backend": "wia",
                "device_id": self.device_id,
            });
        }
        json!({
            "ok": false,
            "status": "unsupported",
            "backend": "wia",
            "device_id": self.device_id,
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        use serde_json::json;
        let xf = x.clamp(0.0, 1.0);
        let yf = y.clamp(0.0, 1.0);
        if let Ok(mut g) = self.focus.lock() {
            *g = Some((xf, yf));
        }
        if self.simulate {
            // Match mock behavior: ((1.0 - min(1.0, dist*1.4)) * 10000).round() / 10000
            let dist = ((xf - 0.5).powi(2) + (yf - 0.5).powi(2)).sqrt();
            let value = ((1.0 - (dist * 1.4).min(1.0)) * 10000.0).round() / 10000.0;
            return json!({
                "ok": true,
                "status": "focused",
                "x_frac": xf,
                "y_frac": yf,
                "focus": value,
                "backend": "wia",
            });
        }
        json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": xf,
            "y_frac": yf,
            "focus": 0.0,
            "backend": "wia",
        })
    }
}

/// Open a WIA session for a listed device (or sim). Does **not** permanently refuse open.
pub fn open(device_id: &str) -> Result<WiaDeviceSession> {
    let id = device_id.trim();
    if !id.starts_with("wia:") && id != "wia" {
        return Err(ScanError::DeviceNotFound(format!(
            "not a WIA device id: {id}"
        )));
    }
    if id == "wia:sim" || (simulate_backends() && id.ends_with(":sim")) {
        return Ok(WiaDeviceSession::new(id.into(), "sim".into(), true));
    }
    let listed = list_devices();
    if !listed.iter().any(|d| d.id == id) {
        return Err(ScanError::DeviceNotFound(format!(
            "unknown WIA device: {id}"
        )));
    }
    let raw = id.strip_prefix("wia:").unwrap_or(id).to_string();
    Ok(WiaDeviceSession::new(id.into(), raw, false))
}
