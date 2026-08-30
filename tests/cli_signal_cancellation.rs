#![cfg(unix)]

use open_scanline::core::{ImageBuffer, PixelFormat};
use open_scanline::imaging::save_image;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn sigint_cancels_cli_jxl_conversion_and_reaps_its_descendant() {
    let directory = scratch_directory("cli-sigint");
    let tools = directory.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let source = directory.join("source.png");
    let destination = directory.join("output.jxl");
    let parent_pid_path = directory.join("parent.pid");
    let descendant_pid_path = directory.join("descendant.pid");
    let late_artifact = directory.join("late-artifact");
    save_image(
        &source,
        &ImageBuffer::new(2, 2, PixelFormat::Rgb8, vec![255; 12]).unwrap(),
        None,
        None,
    )
    .unwrap();
    write_hanging_cjxl(&tools.join("cjxl"));

    let path = std::env::join_paths(
        std::iter::once(tools.clone()).chain(
            std::env::var_os("PATH")
                .as_deref()
                .into_iter()
                .flat_map(std::env::split_paths),
        ),
    )
    .unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args([
            "convert",
            "--in",
            source.to_str().unwrap(),
            "--out",
            destination.to_str().unwrap(),
        ])
        .env("PATH", path)
        .env("CLI_SIGNAL_PARENT_PID", &parent_pid_path)
        .env("CLI_SIGNAL_DESCENDANT_PID", &descendant_pid_path)
        .env("CLI_SIGNAL_LATE_ARTIFACT", &late_artifact)
        .spawn()
        .unwrap();

    let descendant = wait_for_pid(&descendant_pid_path);
    // The fixture can publish its PID while `Command::spawn` is still
    // returning from the fork/exec handshake. Let the CLI enter its
    // supervisor loop before delivering the signal this test is about.
    std::thread::sleep(Duration::from_millis(100));
    let signal_result = unsafe { libc::kill(cli.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(signal_result, 0, "could not send SIGINT to the CLI process");
    let status = cli.wait().unwrap();

    assert_eq!(
        status.code(),
        Some(130),
        "interrupted CLI command must use the conventional SIGINT exit code"
    );
    assert_pid_gone(wait_for_pid(&parent_pid_path));
    assert_pid_gone(descendant);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !late_artifact.exists(),
        "a killed cjxl descendant wrote a late artifact"
    );
    assert!(
        !destination.exists(),
        "interrupted conversion published its destination"
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn sigint_cancels_plugin_sane_discovery_and_reaps_its_descendant() {
    let directory = scratch_directory("plugin-discovery-sigint");
    let tools = directory.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let parent_pid_path = directory.join("parent.pid");
    let descendant_pid_path = directory.join("descendant.pid");
    let late_artifact = directory.join("late-artifact");
    write_hanging_scanimage(&tools.join("scanimage"));

    let mut cli = Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args(["plugin", "--quiet"])
        .env("PATH", path_with_tools(&tools))
        // This regression exercises SANE child-process containment, not
        // network discovery timing. A cold mDNS/subnet pass can legitimately
        // consume the fixture's PID deadline under a loaded full-suite run.
        .env("OPEN_SCANLINE_NETWORK_DISCOVERY", "0")
        .env("CLI_SIGNAL_PARENT_PID", &parent_pid_path)
        .env("CLI_SIGNAL_DESCENDANT_PID", &descendant_pid_path)
        .env("CLI_SIGNAL_LATE_ARTIFACT", &late_artifact)
        .spawn()
        .unwrap();

    let descendant = wait_for_pid(&descendant_pid_path);
    // The script can publish its PID just before the parent's spawn call has
    // fully returned. Wait until the CLI is polling its cancellation token.
    std::thread::sleep(Duration::from_millis(100));
    let signal_result = unsafe { libc::kill(cli.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(signal_result, 0, "could not send SIGINT to the CLI process");
    let status = cli.wait().unwrap();

    assert_eq!(
        status.code(),
        Some(130),
        "plugin exited via signal {:?}",
        status.signal()
    );
    assert_pid_gone(wait_for_pid(&parent_pid_path));
    assert_pid_gone(descendant);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !late_artifact.exists(),
        "a killed scanimage discovery descendant wrote a late artifact"
    );
    let _ = fs::remove_dir_all(directory);
}

fn scratch_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "open-scanline-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_hanging_cjxl(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
printf '%s\n' "$$" > "$CLI_SIGNAL_PARENT_PID"
sh -c '
  trap "" INT TERM
  printf "%s\n" "$$" > "$CLI_SIGNAL_DESCENDANT_PID"
  sleep 1
  : > "$CLI_SIGNAL_LATE_ARTIFACT"
  while :; do sleep 1; done
' &
while :; do sleep 1; done
"#,
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn write_hanging_scanimage(path: &Path) {
    fs::write(
        path,
        r#"#!/bin/sh
printf '%s\n' "$$" > "$CLI_SIGNAL_PARENT_PID"
sh -c '
  trap "" INT TERM
  printf "%s\n" "$$" > "$CLI_SIGNAL_DESCENDANT_PID"
  sleep 1
  : > "$CLI_SIGNAL_LATE_ARTIFACT"
  while :; do sleep 1; done
' &
while :; do sleep 1; done
"#,
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn path_with_tools(tools: &Path) -> std::ffi::OsString {
    std::env::join_paths(
        std::iter::once(tools.to_path_buf()).chain(
            std::env::var_os("PATH")
                .as_deref()
                .into_iter()
                .flat_map(std::env::split_paths),
        ),
    )
    .unwrap()
}

fn wait_for_pid(path: &Path) -> libc::pid_t {
    // Loaded CI runners can take several seconds to reach scanner discovery
    // after the CLI process starts. The PID file is the readiness signal, so
    // keep polling it without weakening the later cancellation deadlines.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(pid) = fs::read_to_string(path).and_then(|pid| {
            pid.trim()
                .parse::<libc::pid_t>()
                .map_err(std::io::Error::other)
        }) {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "fixture did not write a PID to {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn assert_pid_gone(pid: libc::pid_t) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if unsafe { libc::kill(pid, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return;
        }
        assert!(Instant::now() < deadline, "process {pid} remained alive");
        std::thread::sleep(Duration::from_millis(10));
    }
}
