//! Plugin/host mode entry (OSL-PLUGIN) — headless host contract.

use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::{
    list_all_devices_with_cancellation, list_backends_with_cancellation,
};
use crate::infrastructure::config::json::{default_config_path, load_config};
use crate::infrastructure::config::AppConfig;
use crate::infrastructure::runtime::platform::platform_summary;
use crate::operation::CancellationToken;

use crate::workflows::capture::single::{run_scan_to_file, CaptureOptions, ScanToFileArgs};
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
    let config_error = load_config(config_path)
        .err()
        .map(|error| error.to_string());
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
    let mut status = json!({
        "mode": "plugin",
        "app": APP_NAME,
        "version": VERSION,
        "config_path": cfg_path.display().to_string(),
        "devices": devices,
        "backends": backends,
        "shared_scan": "open_scanline::scan::run_scan_to_file",
        "platform": platform_summary(),
        "ok": config_error.is_none(),
    });
    if let (Some(error), Some(obj)) = (config_error, status.as_object_mut()) {
        obj.insert("error".into(), json!(format!("config load: {error}")));
    }
    status
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
    check_plugin_cancellation(cancellation)?;
    let device = selected_plugin_device(device, &cfg);
    let defaults = crate::workflows::settings::resolve_defaults(&cfg);
    let path = run_scan_to_file(
        plugin_scan_args(out_path, &device, defaults, cfg),
        CaptureOptions {
            cancellation: cancellation.clone(),
            ..CaptureOptions::default()
        },
    )?;
    let bytes = std::fs::metadata(&path)?.len();
    Ok(PluginScan {
        path,
        bytes,
        device,
    })
}

fn selected_plugin_device(device: Option<&str>, config: &AppConfig) -> String {
    device
        .map(str::to_owned)
        .or_else(|| Some(config.last_device_id.clone()).filter(|id| !id.is_empty()))
        .unwrap_or_else(|| "mock".into())
}

fn plugin_scan_args(
    out_path: &Path,
    device: &str,
    defaults: crate::workflows::settings::ResolvedSettings,
    config: AppConfig,
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
        overwrite: true,
        config: Some(config),
        ..ScanToFileArgs::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_comes_from_arg_then_config_then_mock() {
        let mut cfg = AppConfig::default();
        assert_eq!(selected_plugin_device(None, &cfg), "mock");
        cfg.last_device_id = "file:x.png".into();
        assert_eq!(selected_plugin_device(None, &cfg), "file:x.png");
        assert_eq!(selected_plugin_device(Some("a"), &cfg), "a");
    }

    #[test]
    fn scan_args_carry_the_config() {
        let cfg = AppConfig {
            invert_colors: true,
            ..AppConfig::default()
        };
        let defaults = crate::workflows::settings::resolve_defaults(&cfg);
        let args = plugin_scan_args(Path::new("o.png"), "mock", defaults, cfg);
        assert!(args.config.is_some_and(|c| c.invert_colors));
    }

    #[test]
    fn corrupt_config_makes_status_not_ok() {
        // The path points at a directory, so loading fails without a file to clean up.
        let path = std::env::temp_dir();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let status = plugin_status_with_cancellation(Some(&path), Some(&cancel));
        assert_eq!(status["ok"], false);
        assert!(status["error"].as_str().is_some());
        assert_eq!(plugin_status_code(&status), 1);
    }
}
