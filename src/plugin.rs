//! Plugin/host mode entry (OSL-PLUGIN) — headless host contract.

use crate::config::{default_config_path, load_config};
use crate::core::{PipelinePrefs, Result, ScanError};
use crate::device::{
    list_all_devices_with_cancellation, list_backends_with_cancellation, CancellationToken,
};
use crate::platform::platform_summary;
use crate::scan::{run_scan_to_file_with_token, ScanToFileArgs};
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
        .map(|p| p.to_path_buf())
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
    if cancellation.is_cancelled() {
        return Err(ScanError::Cancelled("plugin cancelled".into()));
    }
    let mut status = plugin_status_with_cancellation(config_path, Some(&cancellation));
    if cancellation.is_cancelled() {
        return Err(ScanError::Cancelled("plugin cancelled".into()));
    }
    if let Some(out_path) = out {
        let cfg = load_config(config_path)
            .map_err(|error| crate::core::ScanError::Other(format!("config load: {error}")))?;
        let devices = list_all_devices_with_cancellation(Some(&cancellation));
        if cancellation.is_cancelled() {
            return Err(ScanError::Cancelled("plugin cancelled".into()));
        }
        let dev = device
            .map(|s| s.to_string())
            .or_else(|| devices.first().map(|d| d.id.clone()))
            .unwrap_or_else(|| "mock".into());
        let prefs = PipelinePrefs {
            rotate: crate::core::Rotate::from_degrees(cfg.rotate),
            flip_h: cfg.flip_h,
            flip_v: cfg.flip_v,
            brightness: cfg.brightness,
            contrast: cfg.contrast,
            desaturate: cfg.desaturate,
            levels_black: cfg.levels_black,
            levels_white: cfg.levels_white,
            levels_gamma: cfg.levels_gamma,
            ..PipelinePrefs::default()
        };
        let path = run_scan_to_file_with_token(
            ScanToFileArgs {
                device: Some(dev.clone()),
                out: out_path.to_path_buf(),
                width: cfg.default_width,
                height: cfg.default_height,
                dpi: cfg.default_dpi,
                pipeline: prefs,
                ..ScanToFileArgs::default()
            },
            cancellation,
        )?;
        let bytes = std::fs::metadata(&path)?.len();
        if let Some(obj) = status.as_object_mut() {
            obj.insert("scanned".into(), json!(path.display().to_string()));
            obj.insert("bytes".into(), json!(bytes));
            obj.insert("device".into(), json!(dev));
        }
    }
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&status)?);
    }
    let ok = status.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(if ok { 0 } else { 1 })
}
