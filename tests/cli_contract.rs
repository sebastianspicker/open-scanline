use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args(arguments)
        .output()
        .expect("open-scanline binary should start")
}

fn run_isolated_plugin_status() -> Output {
    let config = std::env::temp_dir().join(format!(
        "open-scanline-cli-contract-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args([
            "--config",
            config.to_str().expect("temporary path is UTF-8"),
            "plugin",
        ])
        // Keep this contract independent of locally attached scanners and network state.
        .env("OPEN_SCANLINE_NETWORK_DISCOVERY", "0")
        .env("OPEN_SCANLINE_SKIP_WIA", "1")
        .env("PATH", PathBuf::new())
        .output()
        .expect("plugin status binary should start");
    let _ = std::fs::remove_file(config);
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_help_and_manual_remain_stable_entry_points() {
    let version = run_cli(&["--version"]);
    assert!(version.status.success(), "stderr: {}", stderr(&version));
    assert!(stdout(&version).contains(env!("CARGO_PKG_VERSION")));

    let help = run_cli(&["--help"]);
    assert!(help.status.success(), "stderr: {}", stderr(&help));
    let help_text = stdout(&help);
    assert!(help_text.contains("Usage:"));
    assert!(help_text.contains("scan"));
    assert!(help_text.contains("info"));

    let manual = run_cli(&["help-text"]);
    assert!(manual.status.success(), "stderr: {}", stderr(&manual));
    let manual_text = stdout(&manual);
    assert!(manual_text.contains("Commands:"));
    assert!(manual_text.contains("open_scanline::scan::run_scan_to_file"));
}

#[test]
fn info_and_plugin_status_have_machine_readable_stable_fields() {
    let info = run_cli(&["info", "--module", "ml"]);
    assert!(info.status.success(), "stderr: {}", stderr(&info));
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).expect("info is JSON");
    assert_eq!(info["app"], "open-scanline");
    assert!(info["version"].is_string());
    assert!(info["ml"].is_object());
    assert!(info["ml"]["ok"].is_boolean());

    let plugin = run_isolated_plugin_status();
    assert!(plugin.status.success(), "stderr: {}", stderr(&plugin));
    let plugin: serde_json::Value =
        serde_json::from_slice(&plugin.stdout).expect("plugin status is JSON");
    assert_eq!(plugin["mode"], "plugin");
    assert_eq!(plugin["app"], "open-scanline");
    assert!(plugin["version"].is_string());
    assert!(plugin["ok"].is_boolean());
    assert!(plugin["devices"].is_array());
    assert!(plugin["backends"].is_array());
    assert!(plugin["platform"].is_object());
    for backend in plugin["backends"].as_array().expect("backends is an array") {
        assert!(backend["id"].is_string());
        assert!(backend["name"].is_string());
        assert!(backend["available"].is_boolean());
    }
}

#[test]
fn invalid_arguments_keep_clap_failure_exit_contract() {
    let invalid = run_cli(&["definitely-not-a-command"]);
    assert_eq!(invalid.status.code(), Some(2));
    let message = stderr(&invalid);
    assert!(message.contains("unrecognized subcommand"));
    assert!(message.contains("definitely-not-a-command"));
}
