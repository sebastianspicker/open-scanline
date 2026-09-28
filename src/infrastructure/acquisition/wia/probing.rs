use crate::infrastructure::acquisition::command_backend::system_command_runner;
use crate::infrastructure::acquisition::{
    parse_pipe_devices, simulate_backends, BackendInfo, DeviceInfo,
};
use crate::infrastructure::runtime::{CommandRunner, CommandSpec};
use crate::operation::CancellationToken;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

system_command_runner!(
    /// Production command runner for WIA's PowerShell-driven commands.
    "WIA command",
    "WIA scan cancelled"
);

pub(super) fn powershell_spec(script: String) -> CommandSpec {
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
