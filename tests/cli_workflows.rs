//! End-to-end CLI characterization: pins the externally observable behavior
//! of the `open-scanline` binary across its main subcommands. Every scratch
//! directory and config path is unique per test, so a real user config or
//! shared fixture is never touched.

use open_scanline::imaging::load_image;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-cli-workflows-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("scratch directory should be unique");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// A config path inside the scratch directory. The file does not exist
    /// until a command writes it, so every invocation starts from defaults
    /// instead of a real user config.
    fn config_path(&self) -> PathBuf {
        self.0.join("config.json")
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_open-scanline"));
    command.args(args);
    command
}

/// Isolate a command from locally attached scanners and network discovery, and
/// from any `scanimage`/PowerShell executable that would make backend
/// availability depend on the host machine.
fn isolate(command: &mut Command) {
    command
        .env("OPEN_SCANLINE_NETWORK_DISCOVERY", "0")
        .env("OPEN_SCANLINE_SKIP_WIA", "1")
        .env("PATH", PathBuf::new());
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn tiff_page_count(path: &Path) -> usize {
    let file = fs::File::open(path).expect("tiff output should open");
    let mut decoder = tiff::decoder::Decoder::new(file).expect("tiff output should decode");
    let mut pages = 1;
    while decoder.more_images() {
        decoder.next_image().expect("tiff page should decode");
        pages += 1;
    }
    pages
}

#[test]
fn scan_with_mock_device_writes_the_documented_deterministic_gradient() {
    let directory = ScratchDirectory::new("scan");
    let config = directory.config_path();
    let out = directory.path().join("scan.png");

    let output = cli(&[
        "--config",
        config.to_str().unwrap(),
        "scan",
        "--device",
        "mock",
        "--out",
        out.to_str().unwrap(),
        "--width",
        "3",
        "--height",
        "2",
        "--seed",
        "7",
    ])
    .output()
    .expect("scan should start");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("wrote"));
    assert!(out.is_file());

    let image = load_image(&out).expect("scan output should decode");
    assert_eq!(image.width, 3);
    assert_eq!(image.height, 2);
    assert_eq!(
        image.data,
        vec![
            0, 0, 7, 127, 0, 7, 255, 0, 7, // row y=0
            0, 255, 7, 127, 255, 7, 255, 255, 7, // row y=1
        ]
    );
}

#[test]
fn batch_with_mock_device_writes_pages_in_order_and_an_extension_driven_multipage_tiff() {
    let directory = ScratchDirectory::new("batch");
    let config = directory.config_path();
    let out_dir = directory.path().join("pages");
    let multipage_out = directory.path().join("pages.tif");

    let output = cli(&[
        "--config",
        config.to_str().unwrap(),
        "batch",
        "--device",
        "mock",
        "--out-dir",
        out_dir.to_str().unwrap(),
        "--pages",
        "3",
        "--width",
        "2",
        "--height",
        "1",
        "--seed",
        "10",
        "--multipage-out",
        multipage_out.to_str().unwrap(),
    ])
    .output()
    .expect("batch should start");

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let mut page_names: Vec<String> = fs::read_dir(&out_dir)
        .expect("page directory should exist")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    page_names.sort();
    assert_eq!(
        page_names,
        vec!["page_001.png", "page_002.png", "page_003.png"]
    );

    for (index, name) in page_names.iter().enumerate() {
        let page = load_image(out_dir.join(name)).expect("batch page should decode");
        assert_eq!(page.width, 2);
        assert_eq!(page.height, 1);
        let seed = 10 + index as u8;
        assert_eq!(page.data, vec![0, 0, seed, 255, 0, seed]);
    }

    assert!(multipage_out.is_file());
    assert_eq!(tiff_page_count(&multipage_out), 3);
}

#[test]
fn process_rotate_swaps_dimensions_of_a_scanned_image() {
    let directory = ScratchDirectory::new("process");
    let config = directory.config_path();
    let source = directory.path().join("source.png");
    let processed = directory.path().join("processed.png");

    let scan = cli(&[
        "--config",
        config.to_str().unwrap(),
        "scan",
        "--device",
        "mock",
        "--out",
        source.to_str().unwrap(),
        "--width",
        "4",
        "--height",
        "3",
    ])
    .output()
    .expect("scan should start");
    assert!(scan.status.success(), "stderr: {}", stderr(&scan));

    let process = cli(&[
        "--config",
        config.to_str().unwrap(),
        "process",
        "--in",
        source.to_str().unwrap(),
        "--out",
        processed.to_str().unwrap(),
        "--rotate",
        "90",
    ])
    .output()
    .expect("process should start");

    assert!(process.status.success(), "stderr: {}", stderr(&process));
    assert!(processed.is_file());

    let image = load_image(&processed).expect("processed output should decode");
    assert_eq!(image.width, 3);
    assert_eq!(image.height, 4);
}

#[test]
fn convert_changes_the_container_format_and_preserves_dimensions() {
    let directory = ScratchDirectory::new("convert");
    let config = directory.config_path();
    let source = directory.path().join("source.png");
    let converted = directory.path().join("converted.jpg");

    let scan = cli(&[
        "--config",
        config.to_str().unwrap(),
        "scan",
        "--device",
        "mock",
        "--out",
        source.to_str().unwrap(),
        "--width",
        "5",
        "--height",
        "4",
    ])
    .output()
    .expect("scan should start");
    assert!(scan.status.success(), "stderr: {}", stderr(&scan));

    let convert = cli(&[
        "convert",
        "--in",
        source.to_str().unwrap(),
        "--out",
        converted.to_str().unwrap(),
    ])
    .output()
    .expect("convert should start");

    assert!(convert.status.success(), "stderr: {}", stderr(&convert));
    assert!(converted.is_file());
    assert_eq!(
        open_scanline::imaging::detect_format(&converted).unwrap(),
        Some("jpeg")
    );

    let image = load_image(&converted).expect("converted output should decode");
    assert_eq!(image.width, 5);
    assert_eq!(image.height, 4);
}

#[test]
fn devices_lists_the_always_available_mock_backend_and_device() {
    let mut command = cli(&["devices"]);
    isolate(&mut command);
    let output = command.output().expect("devices should start");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("# backend\tmock\topen-scanline Synthetic Scanner\tavailable=true"));
    assert!(text.contains("mock\topen-scanline Synthetic Scanner\tmock\tmanufacturer=-"));
}

#[test]
fn manufacturers_prints_a_non_empty_catalog() {
    let output = cli(&["manufacturers"])
        .output()
        .expect("manufacturers should start");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("manufacturer-family hints only"));
    assert!(text.contains("epson"));
}

#[test]
fn config_init_and_set_dpi_round_trip_through_show() {
    let directory = ScratchDirectory::new("config");
    let config = directory.config_path();
    assert!(!config.is_file());

    let init = cli(&["--config", config.to_str().unwrap(), "config", "--init"])
        .output()
        .expect("config --init should start");
    assert!(init.status.success(), "stderr: {}", stderr(&init));
    assert!(config.is_file());

    let set_dpi = cli(&[
        "--config",
        config.to_str().unwrap(),
        "config",
        "--set-dpi",
        "222",
    ])
    .output()
    .expect("config --set-dpi should start");
    assert!(set_dpi.status.success(), "stderr: {}", stderr(&set_dpi));

    let show = cli(&["--config", config.to_str().unwrap(), "config", "--show"])
        .output()
        .expect("config --show should start");
    assert!(show.status.success(), "stderr: {}", stderr(&show));
    let text = stdout(&show);
    assert!(text.starts_with(&format!("config_path={}", config.display())));
    assert!(text.contains("\"default_dpi\": 222"));
}

#[test]
fn process_on_a_missing_source_fails_with_exit_code_one() {
    let directory = ScratchDirectory::new("process-failure");
    let config = directory.config_path();
    let missing_source = directory.path().join("does-not-exist.png");
    let destination = directory.path().join("out.png");

    let output = cli(&[
        "--config",
        config.to_str().unwrap(),
        "process",
        "--in",
        missing_source.to_str().unwrap(),
        "--out",
        destination.to_str().unwrap(),
    ])
    .output()
    .expect("process should start");

    assert_eq!(output.status.code(), Some(1));
    assert!(!stderr(&output).is_empty());
    assert!(!destination.exists());
}

#[test]
fn plugin_status_json_has_the_documented_stable_top_level_keys() {
    let directory = ScratchDirectory::new("plugin");
    let config = directory.config_path();

    let mut command = cli(&["--config", config.to_str().unwrap(), "plugin"]);
    isolate(&mut command);
    let output = command.output().expect("plugin should start");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let status: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin status is JSON");
    let object = status.as_object().expect("plugin status is a JSON object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "app",
            "backends",
            "config_path",
            "devices",
            "mode",
            "ok",
            "platform",
            "shared_scan",
            "version",
        ]
    );
    assert_eq!(status["mode"], "plugin");
    assert_eq!(status["app"], "open-scanline");
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["ok"], true);
    assert_eq!(
        status["shared_scan"],
        "open_scanline::scan::run_scan_to_file"
    );
    assert!(status["devices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|device| device["id"] == "mock"));
}
