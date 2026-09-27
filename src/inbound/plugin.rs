//! Plugin/host mode entry (OSL-PLUGIN) — headless host contract.

use crate::error::{Result, ScanError};
use crate::inbound::api::scan::{run_scan_to_file_with_token, ScanToFileArgs};
use crate::infrastructure::acquisition::{
    list_all_devices_with_cancellation, list_backends_with_cancellation,
};
use crate::infrastructure::config::json::{default_config_path, load_config};
use crate::infrastructure::runtime::platform::platform_summary;
use crate::workflows::operation::CancellationToken;
use crate::{APP_NAME, VERSION};
use serde_json::{json, Value};
use std::path::Path;

/// Non-interactive plugin-mode payload for hosts/CI.
pub fn plugin_status(config_path: Option<&Path>) -> Value {
    plugin_status_with_cancellation(config_path, None)
}

/// Plugin status with cancellation for command-backed device discovery.
pub fn plugin_status_with_cancellation(
    config_path: Option<&Path>,
    cancellation: Option<&CancellationToken>,
) -> Value {
    let cfg_path = config_path
        .map(|path| path.to_path_buf())
        .unwrap_or_else(default_config_path);
    let _ = load_config(config_path);
    let devices: Vec<Value> = list_all_devices_with_cancellation(cancellation)
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "kind": d.kind,
            })
        })
        .collect();
    let backends: Vec<Value> = list_backends_with_cancellation(cancellation)
        .into_iter()
        .map(|b| {
            json!({
                "id": b.id,
                "name": b.name,
                "available": b.available,
            })
        })
        .collect();
    json!({
        "mode": "plugin",
        "app": APP_NAME,
        "version": VERSION,
        "config_path": cfg_path.display().to_string(),
        "devices": devices,
        "backends": backends,
        "shared_scan": "open_scanline::scan::run_scan_to_file",
        "platform": platform_summary(),
        "ok": true,
    })
}

/// Headless host entry: print status JSON and optionally run one shared-path scan.
pub fn run_plugin_mode(
    config_path: Option<&Path>,
    out: Option<&Path>,
    quiet: bool,
    device: Option<&str>,
) -> i32 {
    run_plugin_mode_with_token(config_path, out, quiet, device, CancellationToken::new())
}

/// Token-aware plugin entry for hosts that need to interrupt acquisition safely.
pub fn run_plugin_mode_with_token(
    config_path: Option<&Path>,
    out: Option<&Path>,
    quiet: bool,
    device: Option<&str>,
    cancellation: CancellationToken,
) -> i32 {
    match run_plugin_mode_inner_with_token(config_path, out, quiet, device, cancellation) {
        Ok(code) => code,
        Err(ScanError::Cancelled(error)) => {
            eprintln!("plugin error: {error}");
            130
        }
        Err(e) => {
            eprintln!("plugin error: {e}");
            1
        }
    }
}

fn run_plugin_mode_inner_with_token(
    config_path: Option<&Path>,
    out: Option<&Path>,
    quiet: bool,
    device: Option<&str>,
    cancellation: CancellationToken,
) -> Result<i32> {
    check_plugin_cancellation(&cancellation)?;
    let mut status = plugin_status_with_cancellation(config_path, Some(&cancellation));
    check_plugin_cancellation(&cancellation)?;
    if let Some(scan) = optional_plugin_scan(config_path, out, device, &cancellation)? {
        add_plugin_scan_status(&mut status, scan);
    }
    print_plugin_status(&status, quiet)?;
    Ok(plugin_status_code(&status))
}

fn add_plugin_scan_status(status: &mut Value, scan: PluginScan) {
    if let Some(obj) = status.as_object_mut() {
        obj.insert("scanned".into(), json!(scan.path.display().to_string()));
        obj.insert("bytes".into(), json!(scan.bytes));
        obj.insert("device".into(), json!(scan.device));
    }
}

fn print_plugin_status(status: &Value, quiet: bool) -> Result<()> {
    if !quiet {
        println!("{}", serde_json::to_string_pretty(status)?);
    }
    Ok(())
}

fn plugin_status_code(status: &Value) -> i32 {
    if status.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        0
    } else {
        1
    }
}

fn check_plugin_cancellation(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(ScanError::Cancelled("plugin cancelled".into()));
    }
    Ok(())
}

struct PluginScan {
    path: std::path::PathBuf,
    bytes: u64,
    device: String,
}

fn optional_plugin_scan(
    config_path: Option<&Path>,
    out: Option<&Path>,
    device: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<Option<PluginScan>> {
    let Some(out_path) = out else {
        return Ok(None);
    };
    perform_plugin_scan(config_path, out_path, device, cancellation).map(Some)
}

fn perform_plugin_scan(
    config_path: Option<&Path>,
    out_path: &Path,
    device: Option<&str>,
    cancellation: &CancellationToken,
) -> Result<PluginScan> {
    let cfg = load_config(config_path)
        .map_err(|error| ScanError::Other(format!("config load: {error}")))?;
    let devices = list_all_devices_with_cancellation(Some(cancellation));
    check_plugin_cancellation(cancellation)?;
    let device = selected_plugin_device(device, &devices);
    let defaults = crate::workflows::settings::resolve_defaults(&cfg);
    let path = run_scan_to_file_with_token(
        plugin_scan_args(out_path, &device, defaults),
        cancellation.clone(),
    )?;
    let bytes = std::fs::metadata(&path)?.len();
    Ok(PluginScan {
        path,
        bytes,
        device,
    })
}

fn selected_plugin_device(
    device: Option<&str>,
    devices: &[crate::workflows::ports::acquisition::DeviceInfo],
) -> String {
    device
        .map(str::to_owned)
        .or_else(|| devices.first().map(|device| device.id.clone()))
        .unwrap_or_else(|| "mock".into())
}

fn plugin_scan_args(
    out_path: &Path,
    device: &str,
    defaults: crate::workflows::settings::ResolvedSettings,
) -> ScanToFileArgs {
    ScanToFileArgs {
        device: Some(device.to_owned()),
        out: out_path.to_path_buf(),
        width: defaults.acquisition.width,
        height: defaults.acquisition.height,
        dpi: defaults.acquisition.dpi_x,
        mode: defaults.acquisition.mode,
        duplex: defaults.acquisition.duplex,
        pipeline: defaults.processing,
        ..ScanToFileArgs::default()
    }
}
