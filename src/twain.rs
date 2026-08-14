//! TWAIN / host plugin launcher shim — launches open-scanline plugin mode, no proprietary DS.

use crate::platform::platform_summary;
use crate::plugin::run_plugin_mode;
use crate::{APP_NAME, VERSION};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Module capability report for hosts and tests (always safe / non-fatal).
pub fn twain_shim_info() -> Value {
    json!({
        "module": "open_scanline::twain",
        "app": APP_NAME,
        "version": VERSION,
        "ships_native_ds": false,
        "launches_open_scanline_plugin": true,
        "available": true,
        "platform": platform_summary(),
        "ok": true,
    })
}

/// Build argv for host-driven open-scanline launch (plugin mode).
pub fn resolve_host_command(mode: &str, out: Option<&Path>, config: Option<&Path>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(c) = config {
        args.push("--config".into());
        args.push(c.display().to_string());
    }
    match mode {
        "gui" => args.push("gui".into()),
        "plugin" => {
            args.push("plugin".into());
            if let Some(o) = out {
                args.push("--out".into());
                args.push(o.display().to_string());
            }
        }
        other => args.push(other.into()),
    }
    args
}

fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("open-scanline"))
}

/// Launch open-scanline in plugin mode for a host application.
///
/// When `wait` is true, prefers in-process `run_plugin_mode` for reliability,
/// falling back to spawning the current binary.
pub fn launch_plugin_host(
    config: Option<&Path>,
    out: Option<&Path>,
    wait: bool,
    quiet: bool,
) -> Value {
    let argv = resolve_host_command("plugin", out, config);
    if wait {
        // In-process path (same orchestration as CLI plugin).
        let code = run_plugin_mode(config, out, quiet, None);
        return json!({
            "argv": argv,
            "returncode": code,
            "ok": code == 0,
            "version": VERSION,
            "launcher": "twain.launch_plugin_host",
            "in_process": true,
        });
    }
    let exe = current_exe();
    let mut cmd = Command::new(&exe);
    cmd.args(&argv);
    match cmd.spawn() {
        Ok(child) => json!({
            "argv": argv,
            "pid": child.id(),
            "ok": true,
            "version": VERSION,
            "launcher": "twain.launch_plugin_host",
            "in_process": false,
        }),
        Err(e) => json!({
            "argv": argv,
            "ok": false,
            "error": e.to_string(),
            "version": VERSION,
            "launcher": "twain.launch_plugin_host",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_plugin_host_in_process() {
        let v = launch_plugin_host(None, None, true, true);
        assert_eq!(v["ok"], true);
        assert_eq!(v["launcher"], "twain.launch_plugin_host");
    }

    #[test]
    fn resolve_host_command_plugin() {
        let args = resolve_host_command("plugin", Some(Path::new("out.png")), None);
        assert!(args.iter().any(|a| a == "plugin"));
        assert!(args.iter().any(|a| a == "out.png"));
    }
}
