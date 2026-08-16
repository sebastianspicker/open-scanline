//! TWAIN / host plugin launcher shim — launches open-scanline plugin mode, no proprietary DS.

use crate::platform::platform_summary;
use crate::plugin::run_plugin_mode;
use crate::{APP_NAME, VERSION};
use serde_json::{json, Value};
use std::io;
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

fn current_exe() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    if executable.is_absolute() {
        Ok(executable)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "current executable path is not absolute: {}",
                executable.display()
            ),
        ))
    }
}

fn launch_out_of_process<R, S>(argv: &[String], resolve_executable: R, spawn: S) -> io::Result<u32>
where
    R: FnOnce() -> io::Result<PathBuf>,
    S: FnOnce(&Path, &[String]) -> io::Result<u32>,
{
    let executable = resolve_executable().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not resolve the current executable for plugin launch: {error}"),
        )
    })?;
    spawn(&executable, argv)
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
    match launch_out_of_process(&argv, current_exe, |executable, args| {
        let mut command = Command::new(executable);
        command.args(args);
        command.spawn().map(|child| child.id())
    }) {
        Ok(pid) => json!({
            "argv": argv,
            "pid": pid,
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

    #[test]
    fn out_of_process_launch_does_not_spawn_when_current_executable_resolution_fails() {
        use std::cell::Cell;

        let hostile_path_executable = Path::new("/attacker-controlled/bin/open-scanline");
        let spawn_attempted = Cell::new(false);
        let error = launch_out_of_process(
            &["plugin".to_string()],
            || {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "current executable unavailable",
                ))
            },
            |executable, _| {
                spawn_attempted.set(true);
                assert_ne!(executable, hostile_path_executable);
                Ok(1)
            },
        )
        .expect_err("a failed current executable lookup must not fall back to PATH");

        assert!(error
            .to_string()
            .contains("could not resolve the current executable"));
        assert!(
            !spawn_attempted.get(),
            "a hostile PATH entry must never receive a spawn attempt"
        );
    }
}
