use crate::infrastructure::acquisition::batch::probe_timeout;
use crate::infrastructure::acquisition::command_backend::system_command_runner;
use crate::infrastructure::acquisition::{
    parse_pipe_devices, simulate_backends, BackendInfo, DeviceInfo,
};
use crate::infrastructure::runtime::{CommandRunner, CommandSpec};
use crate::operation::CancellationToken;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

system_command_runner!(
    /// Production command runner for WIA's PowerShell-driven commands.
    "WIA command",
    "WIA scan cancelled"
);

pub(super) fn powershell_spec(script: String) -> CommandSpec {
    CommandSpec {
        program: powershell_program(),
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

/// Windows PowerShell under `%SystemRoot%`. A bare `powershell` is resolved by
/// Windows through the current directory before `PATH`, so a planted
/// `powershell.exe` could run instead.
fn system_powershell_path(system_root: Option<&str>) -> PathBuf {
    let root = system_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .unwrap_or("C:\\Windows");
    PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

/// The absolute system path when that file exists, else the bare name.
fn resolve_powershell(system_root: Option<&str>, exists: impl Fn(&Path) -> bool) -> String {
    let path = system_powershell_path(system_root);
    if exists(&path) {
        path.display().to_string()
    } else {
        POWERSHELL.into()
    }
}

fn powershell_program() -> String {
    resolve_powershell(
        std::env::var("SystemRoot").ok().as_deref(),
        |path: &Path| path.is_file(),
    )
}

/// Check the Windows PowerShell executable rather than reporting an unusable
/// backend when PowerShell has been removed from PATH.
fn powershell_available() -> bool {
    powershell_available_with_cancellation(None)
}

fn powershell_available_with_cancellation(_cancellation: Option<&CancellationToken>) -> bool {
    #[cfg(target_os = "windows")]
    {
        let cancelled = Mutex::new(false);
        let result = SystemCommandRunner.run(
            &CommandSpec {
                program: powershell_program(),
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "$null".into(),
                ],
            },
            probe_timeout(Duration::from_secs(10)),
            &cancelled,
            _cancellation,
            None,
        );
        if let Err(error) = &result {
            crate::debug_log::debug_log!("wia", "PowerShell probe failed: {error}");
        }
        result.is_ok_and(|output| output.success)
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
        crate::debug_log::debug_log!("wia", "PowerShell unavailable; skipping enumeration");
        return Vec::new();
    }
    let cancelled = Mutex::new(false);
    let output = match SystemCommandRunner.run(
        &powershell_spec(wia_enumeration_script().into()),
        probe_timeout(Duration::from_secs(8)),
        &cancelled,
        cancellation,
        None,
    ) {
        Ok(output) => output,
        Err(error) => {
            crate::debug_log::debug_log!("wia", "device enumeration failed: {error}");
            return Vec::new();
        }
    };
    let devices = parse_wia_devices(&String::from_utf8_lossy(&output.stdout));
    crate::debug_log::debug_log!("wia", "parsed {} device(s)", devices.len());
    devices
}

pub fn parse_wia_devices(text: &str) -> Vec<DeviceInfo> {
    parse_pipe_devices(text, "wia", "WIA", "wia")
}

/// List WIA devices. Empty-safe; a non-empty listing is cached. Includes `wia:sim` when simulate env set.
pub fn list_devices() -> Vec<DeviceInfo> {
    list_devices_with_cancellation(None)
}

static CACHE: Mutex<Option<Vec<DeviceInfo>>> = Mutex::new(None);

/// List WIA devices while allowing bounded PowerShell discovery to be cancelled.
pub fn list_devices_with_cancellation(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    if cancellation.is_some() {
        return list_devices_uncached(cancellation);
    }
    let mut devices = cached_listing(&CACHE, || {
        if std::env::var("OPEN_SCANLINE_SKIP_WIA").ok().as_deref() == Some("1") {
            return Vec::new();
        }
        std::panic::catch_unwind(|| try_list_via_powershell(None)).unwrap_or_default()
    });
    if simulate_backends() && !devices.iter().any(|d| d.id == "wia:sim") {
        devices.push(DeviceInfo::new("wia:sim", "WIA Simulated Scanner", "wia"));
    }
    devices
}

/// The cached listing, else a fresh one. An empty result (scanner off, slow
/// PowerShell) is never kept, so the next call looks again.
fn cached_listing(
    cache: &Mutex<Option<Vec<DeviceInfo>>>,
    list: impl FnOnce() -> Vec<DeviceInfo>,
) -> Vec<DeviceInfo> {
    if let Some(devices) = cache.lock().ok().and_then(|cached| cached.clone()) {
        return devices;
    }
    let devices = list();
    if !devices.is_empty() {
        if let Ok(mut cached) = cache.lock() {
            *cached = Some(devices.clone());
        }
    }
    devices
}

pub(super) fn list_devices_uncached(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
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

/// An uncached listing whose real (non-simulated) devices replace the cache,
/// so a scanner found after a miss is not looked up again on the next call.
pub(super) fn list_devices_refreshing_cache() -> Vec<DeviceInfo> {
    let devices = list_devices_uncached(None);
    let real: Vec<DeviceInfo> = devices
        .iter()
        .filter(|device| device.id != "wia:sim")
        .cloned()
        .collect();
    if !real.is_empty() {
        if let Ok(mut cached) = CACHE.lock() {
            *cached = Some(real);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_powershell_path_uses_system_root_or_the_default() {
        let expected = |root: &str| {
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        };
        assert_eq!(system_powershell_path(Some("D:\\Win")), expected("D:\\Win"));
        assert_eq!(system_powershell_path(None), expected("C:\\Windows"));
        assert_eq!(system_powershell_path(Some("  ")), expected("C:\\Windows"));
    }

    fn device(id: &str) -> DeviceInfo {
        DeviceInfo::new(id, "scanner", "wia")
    }

    #[test]
    fn an_empty_listing_is_not_cached() {
        let cache = Mutex::new(None);
        assert!(cached_listing(&cache, Vec::new).is_empty());
        let found = cached_listing(&cache, || vec![device("wia:a")]);
        assert_eq!(found.len(), 1);
        // The non-empty listing is kept and answers without listing again.
        let again = cached_listing(&cache, || panic!("must not list again"));
        assert_eq!(again[0].id, "wia:a");
    }

    #[test]
    fn bare_powershell_is_used_only_when_the_system_file_is_missing() {
        let present = resolve_powershell(Some("C:\\Windows"), |_| true);
        assert!(present.ends_with("powershell.exe") && present != POWERSHELL);
        assert_eq!(
            resolve_powershell(Some("C:\\Windows"), |_| false),
            "powershell"
        );
    }
}
