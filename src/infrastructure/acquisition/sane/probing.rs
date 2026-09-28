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

pub(super) fn which(bin: &str) -> Option<std::path::PathBuf> {
    let Ok(path) = std::env::var("PATH") else {
        return None;
    };
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
