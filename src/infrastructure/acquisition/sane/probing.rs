use crate::infrastructure::acquisition::batch::probe_timeout;
use crate::infrastructure::acquisition::command_backend::system_command_runner;
use crate::infrastructure::acquisition::{
    parse_pipe_devices, simulate_backends, BackendInfo, DeviceInfo,
};
use crate::infrastructure::runtime::{CommandRunner, CommandSpec};
use crate::operation::CancellationToken;
use std::sync::Mutex;
use std::time::Duration;

system_command_runner!(
    /// Production command runner for the SANE `scanimage` tool.
    "scanimage",
    "SANE scan cancelled"
);

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
        let result = SystemCommandRunner.run_with_cancellation(
            &CommandSpec {
                program: binary.display().to_string(),
                args: vec!["--version".into()],
            },
            probe_timeout(Duration::from_secs(2)),
            &cancelled,
            cancellation,
        );
        if let Err(error) = &result {
            crate::debug_log::debug_log!("sane", "scanimage --version probe failed: {error}");
        }
        result.is_ok_and(|output| output.success)
    })
}

/// Package-manager prefixes searched after `PATH`. Apps started from Finder or
/// the Dock get only the system `PATH`, which omits Homebrew and MacPorts.
#[cfg(target_os = "macos")]
const FALLBACK_TOOL_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];
#[cfg(not(target_os = "macos"))]
const FALLBACK_TOOL_DIRS: &[&str] = &[];

pub(super) fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let fallback = FALLBACK_TOOL_DIRS.iter().map(std::path::PathBuf::from);
    let found = find_in_dirs(bin, std::env::split_paths(&path).chain(fallback));
    match &found {
        Some(path) => crate::debug_log::debug_log!("sane", "found {bin} at {}", path.display()),
        None => crate::debug_log::debug_log!("sane", "{bin} not found in PATH or fallback dirs"),
    }
    found
}

fn find_in_dirs(
    bin: &str,
    dirs: impl IntoIterator<Item = std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    for dir in dirs {
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
    let output = match SystemCommandRunner.run_with_cancellation(
        &CommandSpec {
            program: bin.display().to_string(),
            args: vec!["-f".into(), "%d|%v %m%n".into()],
        },
        probe_timeout(Duration::from_secs(8)),
        &cancelled,
        cancellation,
    ) {
        Ok(output) => output,
        Err(error) => {
            crate::debug_log::debug_log!("sane", "device listing failed: {error}");
            return Vec::new();
        }
    };
    let devices = parse_sane_devices(&String::from_utf8_lossy(&output.stdout));
    crate::debug_log::debug_log!("sane", "parsed {} device(s)", devices.len());
    devices
}

pub fn parse_sane_devices(text: &str) -> Vec<DeviceInfo> {
    parse_pipe_devices(text, "sane", "SANE", "sane")
}

/// List SANE devices. Empty-safe. Includes `sane:sim` when simulate env set.
pub fn list_devices() -> Vec<DeviceInfo> {
    list_devices_with_cancellation(None)
}

/// How long a successful listing answers device-id checks when opening a
/// session; enumeration can take seconds and every scan opens a session.
const LISTING_REUSE: Duration = Duration::from_secs(30);

static LAST_LISTING: Mutex<Option<(std::time::Instant, Vec<DeviceInfo>)>> = Mutex::new(None);

/// Devices from a listing at most [`LISTING_REUSE`] old, or a fresh listing.
pub(super) fn recent_devices(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    let cached = LAST_LISTING
        .lock()
        .ok()
        .and_then(|last| last.clone())
        .filter(|(listed_at, _)| listed_at.elapsed() < LISTING_REUSE);
    match cached {
        Some((_, devices)) => devices,
        None => list_devices_with_cancellation(cancellation),
    }
}

/// List SANE devices while allowing bounded `scanimage` discovery to be cancelled.
pub fn list_devices_with_cancellation(cancellation: Option<&CancellationToken>) -> Vec<DeviceInfo> {
    let mut devices =
        std::panic::catch_unwind(|| try_list_scanimage(cancellation)).unwrap_or_default();
    if !devices.is_empty() {
        if let Ok(mut last) = LAST_LISTING.lock() {
            *last = Some((std::time::Instant::now(), devices.clone()));
        }
    }
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

#[cfg(test)]
mod tests {
    use super::find_in_dirs;

    #[test]
    fn tool_lookup_falls_back_to_later_directories() {
        let root = std::env::temp_dir().join(format!("osl-which-{}", std::process::id()));
        let (path_dir, brew_dir) = (root.join("path"), root.join("brew"));
        std::fs::create_dir_all(&path_dir).unwrap();
        std::fs::create_dir_all(&brew_dir).unwrap();
        std::fs::write(brew_dir.join("scanimage"), b"").unwrap();
        let found = find_in_dirs("scanimage", [path_dir.clone(), brew_dir.clone()]);
        assert_eq!(found, Some(brew_dir.join("scanimage")));
        assert_eq!(find_in_dirs("scanimage", [path_dir]), None);
    }
}
