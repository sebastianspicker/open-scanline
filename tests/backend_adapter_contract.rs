#[path = "backend_adapter_contract/sane.rs"]
mod sane_contract;
#[path = "backend_adapter_contract/support.rs"]
mod support;
#[path = "backend_adapter_contract/wia.rs"]
mod wia_contract;

use open_scanline::core::{PixelFormat, ScanMode, ScanRequest};
use open_scanline::device::{
    DeviceSession, FileDeviceSession, FocusCapability, MaintenanceAvailability, MockDeviceSession,
};
use open_scanline::{escl, sane, wia};
use std::path::{Path, PathBuf};

#[test]
fn command_builders_keep_values_as_arguments_or_quoted_script_data() {
    let request = ScanRequest {
        dpi_x: 600,
        dpi_y: 600,
        mode: ScanMode::Reflective,
        pixel_format: PixelFormat::Rgb8,
        width: 100,
        height: 100,
        region: None,
        duplex: false,
        seed: 0,
        ..ScanRequest::default()
    };
    let sane = sane::scanimage_command(
        Path::new("scanimage"),
        "net:scanner;not-a-shell",
        &request,
        Path::new("out;still-data.png"),
    );
    assert_eq!(sane.args[1], "net:scanner;not-a-shell");
    assert!(sane
        .args
        .iter()
        .any(|argument| argument == "--output-file=out;still-data.png"));

    let wia = wia::wia_transfer_command("id'quoted", &request, Path::new("out'quoted.png"));
    assert!(wia.args.last().unwrap().contains("$want = 'id''quoted'"));
    assert!(wia.args.last().unwrap().contains("out''quoted.png"));
    assert!(wia.args.last().unwrap().contains("Value = 600"));
}

#[test]
fn device_parsers_ignore_blank_and_empty_identifier_records() {
    let sane_devices = sane::parse_sane_devices("\n|missing\n/dev/scanner|Vendor Model\n");
    assert_eq!(sane_devices.len(), 1);
    assert_eq!(sane_devices[0].id, "sane:/dev/scanner");
    assert_eq!(sane_devices[0].name, "SANE: Vendor Model");
    assert_eq!(sane_devices[0].kind, "sane");

    let wia_devices = wia::parse_wia_devices("\n|missing\nraw-id|Windows Scanner\n");
    assert_eq!(wia_devices.len(), 1);
    assert_eq!(wia_devices[0].id, "wia:raw-id");
    assert_eq!(wia_devices[0].name, "WIA: Windows Scanner");
    assert_eq!(wia_devices[0].kind, "wia");
}
#[test]
fn sane_maintenance_parser_only_enables_allowlisted_options_and_builds_exact_args() {
    let options = sane::parse_scanimage_maintenance_options(
        "  --calibrate\n  --autofocus\n  --focusx [10..110]\n  --focusy [20..220]\n",
    );
    assert!(options.calibrate);
    assert!(options.autofocus);
    assert_eq!(options.focus_x_range, Some((10.0, 110.0)));
    assert_eq!(options.focus_y_range, Some((20.0, 220.0)));

    assert_point_maintenance_command(&options);
    assert_hostile_maintenance_options();
    assert_centre_maintenance_command();
    assert_non_button_and_nan_options(&options);
    assert_extreme_maintenance_range();
}

fn assert_point_maintenance_command(options: &sane::SaneMaintenanceOptions) {
    let point = sane::scanimage_maintenance_command(
        Path::new("scanimage"),
        "net:scanner;not-a-shell",
        options,
        sane::SaneMaintenanceAction::FocusPoint {
            x_fraction: 0.25,
            y_fraction: 0.5,
        },
    )
    .unwrap();
    assert_eq!(
        point.args,
        [
            "-d",
            "net:scanner;not-a-shell",
            "--focusx",
            "35",
            "--focusy",
            "120",
            "--autofocus",
            "--dont-scan",
        ]
    );
}

fn assert_hostile_maintenance_options() {
    let hostile = sane::parse_scanimage_maintenance_options(
        "--autofocus; --run-this\n--focusx [0..not-a-number]\n--focusy [0..1]\nThe scanner can use --calibrate\nThe scanner can use --autofocus\n",
    );
    assert!(!hostile.calibrate);
    assert!(!hostile.autofocus);
    assert_eq!(hostile.focus_x_range, None);
    assert!(sane::scanimage_maintenance_command(
        Path::new("scanimage"),
        "device",
        &hostile,
        sane::SaneMaintenanceAction::FocusPoint {
            x_fraction: 0.5,
            y_fraction: 0.5,
        },
    )
    .is_err());
}

fn assert_centre_maintenance_command() {
    let centre = sane::parse_scanimage_maintenance_options("  --focus-on-centre\n");
    assert_eq!(
        sane::scanimage_maintenance_command(
            Path::new("scanimage"),
            "device",
            &centre,
            sane::SaneMaintenanceAction::FocusCenter,
        )
        .unwrap()
        .args,
        ["-d", "device", "--focus-on-centre", "--dont-scan"]
    );
}

fn assert_non_button_and_nan_options(options: &sane::SaneMaintenanceOptions) {
    let not_a_button = sane::parse_scanimage_maintenance_options(
        "--calibrate [yes|no]\n--autofocus=yes\n--focusx [0..100]\n--focusy [0..100]\n",
    );
    assert!(!not_a_button.calibrate);
    assert!(!not_a_button.autofocus);
    assert!(sane::scanimage_maintenance_command(
        Path::new("scanimage"),
        "device",
        options,
        sane::SaneMaintenanceAction::FocusPoint {
            x_fraction: f64::NAN,
            y_fraction: 0.5,
        },
    )
    .is_err());
}

fn assert_extreme_maintenance_range() {
    let extreme = sane::parse_scanimage_maintenance_options(&format!(
        "--autofocus\n--focusx [{}..{}]\n--focusy [0..1]\n",
        -f64::MAX,
        f64::MAX
    ));
    let extreme_command = sane::scanimage_maintenance_command(
        Path::new("scanimage"),
        "device",
        &extreme,
        sane::SaneMaintenanceAction::FocusPoint {
            x_fraction: 0.5,
            y_fraction: 0.5,
        },
    )
    .unwrap();
    assert_eq!(extreme_command.args[3], "0");
}

#[test]
fn maintenance_states_are_truthful_for_non_sane_backends() {
    let mock = MockDeviceSession::new();
    assert!(matches!(
        mock.maintenance_capabilities().calibration,
        MaintenanceAvailability::Simulated
    ));
    assert!(matches!(
        mock.maintenance_capabilities().focus,
        FocusCapability::Point {
            availability: MaintenanceAvailability::Simulated,
            ..
        }
    ));

    let file = FileDeviceSession::new(PathBuf::from("unused-maintenance-fixture.png"));
    assert!(!file.maintenance_capabilities().calibration.is_available());
    assert!(!file.maintenance_capabilities().focus.is_available());

    let wia_simulated = wia::WiaDeviceSession::new("wia:sim".into(), "sim".into(), true);
    assert!(matches!(
        wia_simulated.maintenance_capabilities().calibration,
        MaintenanceAvailability::Simulated
    ));
    let wia_real = wia::WiaDeviceSession::new("wia:real".into(), "real".into(), false);
    assert!(!wia_real.maintenance_capabilities().focus.is_available());
    assert_eq!(wia_real.focus(0.25, 0.75)["status"], "unsupported");

    let escl_simulated = escl::open("escl:sim").unwrap();
    assert!(!escl_simulated
        .maintenance_capabilities()
        .calibration
        .is_available());
    assert!(!escl_simulated
        .maintenance_capabilities()
        .focus
        .is_available());
}
