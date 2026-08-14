use super::args::{Cli, Commands, OcrEngineArg};
use super::run;
use crate::config::{save_config, AppConfig};
use crate::core::{ImageBuffer, PixelFormat};
use crate::imaging::{load_image, save_image};
use clap::{CommandFactory, Parser};
use std::fs;

#[test]
fn version_flag_succeeds() {
    let code = run(&["--version".into()]);
    assert_eq!(code, 0);
}

#[test]
fn help_text_succeeds() {
    let code = run(&["help-text".into()]);
    assert_eq!(code, 0);
}

#[test]
fn onnx_help_succeeds() {
    let code = run(&["onnx".into(), "--help".into()]);
    assert_eq!(code, 0);
}

#[test]
fn scan_process_and_batch_help_document_export_controls() {
    for command in ["scan", "process", "batch"] {
        let mut cli = Cli::command();
        let help = cli
            .find_subcommand_mut(command)
            .unwrap()
            .render_long_help()
            .to_string();
        for flag in [
            "--pdf-searchable",
            "--pdf-password",
            "--pdf-password-file",
            "--allow-insecure-password-argv",
            "--ocr-lang",
            "--ocr-engine",
            "--scanner-profile",
        ] {
            assert!(help.contains(flag), "{command} help omits {flag}");
        }
        assert!(help.contains("offline"));
        assert!(help.contains("tesseract"));
    }
}

#[test]
fn batch_help_exposes_the_full_shared_pipeline() {
    let mut cli = Cli::command();
    let help = cli
        .find_subcommand_mut("batch")
        .unwrap()
        .render_long_help()
        .to_string();
    for flag in [
        "--rotate",
        "--crop",
        "--brightness",
        "--saturation",
        "--curves",
        "--levels-gamma",
        "--auto-deskew",
        "--auto-crop",
        "--auto-orient",
        "--white-balance",
        "--auto-levels",
        "--infrared-clean",
        "--descreen-dpi",
        "--film-type",
        "--restore-colors",
        "--restore-fading",
        "--grain-reduction",
        "--flatten",
        "--hole-punch",
        "--colorize-mode",
    ] {
        assert!(help.contains(flag), "batch help omits {flag}");
    }
}

#[test]
fn configurable_booleans_preserve_bare_flags_and_accept_explicit_false() {
    let omitted = Cli::try_parse_from(["open-scanline", "scan", "--out", "scan.png"]).unwrap();
    let enabled = Cli::try_parse_from([
        "open-scanline",
        "scan",
        "--out",
        "scan.png",
        "--duplex",
        "--invert",
        "--auto-crop",
    ])
    .unwrap();
    let disabled = Cli::try_parse_from([
        "open-scanline",
        "scan",
        "--out",
        "scan.png",
        "--duplex=false",
        "--invert=false",
        "--auto-crop=false",
    ])
    .unwrap();

    match omitted.cmd.unwrap() {
        Commands::Scan {
            duplex,
            invert,
            auto_crop,
            ..
        } => assert_eq!((duplex, invert, auto_crop), (None, None, None)),
        _ => panic!("scan command was not parsed"),
    }
    match enabled.cmd.unwrap() {
        Commands::Scan {
            duplex,
            invert,
            auto_crop,
            ..
        } => assert_eq!(
            (duplex, invert, auto_crop),
            (Some(true), Some(true), Some(true))
        ),
        _ => panic!("scan command was not parsed"),
    }
    match disabled.cmd.unwrap() {
        Commands::Scan {
            duplex,
            invert,
            auto_crop,
            ..
        } => assert_eq!(
            (duplex, invert, auto_crop),
            (Some(false), Some(false), Some(false))
        ),
        _ => panic!("scan command was not parsed"),
    }
}

#[test]
fn process_help_exposes_the_full_shared_pipeline() {
    let mut cli = Cli::command();
    let help = cli
        .find_subcommand_mut("process")
        .unwrap()
        .render_long_help()
        .to_string();
    for flag in [
        "--rotate",
        "--crop",
        "--brightness",
        "--saturation",
        "--curves",
        "--levels-black",
        "--levels-white",
        "--levels-gamma",
        "--auto-deskew",
        "--auto-crop",
        "--auto-orient",
        "--white-balance",
        "--auto-levels",
        "--infrared-clean",
        "--descreen",
        "--descreen-dpi",
        "--film-type",
        "--restore-colors",
        "--restore-fading",
        "--grain-reduction",
        "--flatten",
        "--hole-punch",
        "--colorize-mode",
    ] {
        assert!(help.contains(flag), "process help omits {flag}");
    }
}

#[test]
fn process_parses_full_pipeline_override_surface() {
    let cli = Cli::try_parse_from([
        "open-scanline",
        "process",
        "--in",
        "input.png",
        "--out",
        "output.png",
        "--levels-black",
        "3",
        "--levels-white",
        "250",
        "--levels-gamma",
        "1.2",
        "--infrared-clean",
        "medium",
        "--descreen",
        "--descreen-dpi",
        "300",
        "--film-type",
        "negative",
        "--restore-colors",
        "--restore-fading",
        "--grain-reduction",
        "light",
        "--flatten",
        "--hole-punch",
        "--colorize-mode",
        "auto",
    ])
    .unwrap();

    match cli.cmd.unwrap() {
        Commands::Process {
            levels_black,
            levels_white,
            levels_gamma,
            infrared_clean,
            descreen,
            descreen_dpi,
            film_type,
            restore_colors,
            restore_fading,
            grain_reduction,
            flatten,
            hole_punch,
            colorize_mode,
            ..
        } => {
            assert_eq!(levels_black, Some(3));
            assert_eq!(levels_white, Some(250));
            assert_eq!(levels_gamma, Some(1.2));
            assert_eq!(infrared_clean.as_deref(), Some("medium"));
            assert_eq!(descreen, Some(true));
            assert_eq!(descreen_dpi, Some(300));
            assert_eq!(film_type.as_deref(), Some("negative"));
            assert_eq!(restore_colors, Some(true));
            assert_eq!(restore_fading, Some(true));
            assert_eq!(grain_reduction.as_deref(), Some("light"));
            assert_eq!(flatten, Some(true));
            assert_eq!(hole_punch, Some(true));
            assert_eq!(colorize_mode.as_deref(), Some("auto"));
        }
        _ => panic!("process command was not parsed"),
    }
}

#[test]
fn direct_escl_trust_is_an_explicit_scan_and_batch_flag() {
    for command in ["scan", "batch"] {
        let mut cli = Cli::command();
        let help = cli
            .find_subcommand_mut(command)
            .unwrap()
            .render_long_help()
            .to_string();
        assert!(help.contains("--allow-unlisted-escl"));
    }

    let scan = Cli::try_parse_from([
        "open-scanline",
        "scan",
        "--device",
        "escl:scanner.local:8080",
        "--allow-unlisted-escl",
        "--out",
        "scan.png",
    ])
    .unwrap();
    assert!(matches!(
        scan.cmd,
        Some(Commands::Scan {
            allow_unlisted_escl: true,
            ..
        })
    ));

    let batch = Cli::try_parse_from([
        "open-scanline",
        "batch",
        "--device",
        "escl:https@scanner.local:443",
        "--allow-unlisted-escl",
        "--out-dir",
        "pages",
    ])
    .unwrap();
    assert!(matches!(
        batch.cmd,
        Some(Commands::Batch {
            allow_unlisted_escl: true,
            ..
        })
    ));
}

#[test]
fn export_options_parse_for_process_and_batch() {
    let cli = Cli::try_parse_from([
        "open-scanline",
        "process",
        "--in",
        "input.png",
        "--out",
        "output.pdf",
        "--pdf-searchable",
        "--pdf-password-file",
        "password.txt",
        "--ocr-engine",
        "offline",
        "--scanner-profile",
        "profile.json",
    ])
    .unwrap();
    match cli.cmd.unwrap() {
        Commands::Process {
            pdf_searchable,
            pdf_password,
            pdf_password_file,
            ocr_lang,
            ocr_engine,
            scanner_profile,
            ..
        } => {
            assert!(pdf_searchable);
            assert!(pdf_password.is_none());
            assert_eq!(
                pdf_password_file,
                Some(std::path::PathBuf::from("password.txt"))
            );
            assert_eq!(ocr_lang, None);
            assert!(matches!(ocr_engine, Some(OcrEngineArg::Offline)));
            assert_eq!(
                scanner_profile.unwrap(),
                std::path::PathBuf::from("profile.json")
            );
        }
        _ => panic!("process command was not parsed"),
    }

    let cli = Cli::try_parse_from([
        "open-scanline",
        "batch",
        "--out-dir",
        "pages",
        "--multipage-out",
        "combined.pdf",
        "--ocr-engine",
        "tesseract",
    ])
    .unwrap();
    match cli.cmd.unwrap() {
        Commands::Batch {
            multipage_out,
            ocr_engine,
            ocr_lang,
            ..
        } => {
            assert_eq!(
                multipage_out.unwrap(),
                std::path::PathBuf::from("combined.pdf")
            );
            assert!(matches!(ocr_engine, Some(OcrEngineArg::Tesseract)));
            assert_eq!(ocr_lang, None);
        }
        _ => panic!("batch command was not parsed"),
    }
}

#[test]
fn scan_process_and_batch_ocr_flags_are_optional_overrides() {
    for args in [
        vec!["open-scanline", "scan", "--out", "scan.pdf"],
        vec![
            "open-scanline",
            "process",
            "--in",
            "in.png",
            "--out",
            "out.pdf",
        ],
        vec!["open-scanline", "batch", "--out-dir", "pages"],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.cmd.unwrap() {
            Commands::Scan {
                ocr_lang,
                ocr_engine,
                ..
            }
            | Commands::Process {
                ocr_lang,
                ocr_engine,
                ..
            }
            | Commands::Batch {
                ocr_lang,
                ocr_engine,
                ..
            } => {
                assert_eq!(ocr_lang, None);
                assert!(ocr_engine.is_none());
            }
            _ => unreachable!("expected an export command"),
        }
    }
}

#[test]
fn process_cli_writes_searchable_pdf_with_offline_ocr() {
    let directory = std::env::temp_dir().join("open_scanline_cli_searchable_pdf");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let output = directory.join("output.pdf");
    let image = ImageBuffer::new(2, 2, PixelFormat::Rgb8, vec![220; 12]).unwrap();
    save_image(&source, &image, None, None).unwrap();

    let code = run(&[
        "process".into(),
        "--in".into(),
        source.display().to_string(),
        "--out".into(),
        output.display().to_string(),
        "--pdf-searchable".into(),
        "--ocr-engine".into(),
        "offline".into(),
    ]);

    assert_eq!(code, 0);
    assert!(output.is_file());
}

#[test]
fn pdf_only_export_flags_fail_before_non_pdf_output_is_written() {
    let directory = std::env::temp_dir().join("open_scanline_cli_non_pdf_export_flags");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let output = directory.join("output.png");
    let password_file = directory.join("password.txt");
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![220, 220, 220]).unwrap();
    save_image(&source, &image, None, None).unwrap();
    fs::write(&password_file, b"test-password\n").unwrap();

    let code = run(&[
        "process".into(),
        "--in".into(),
        source.display().to_string(),
        "--out".into(),
        output.display().to_string(),
        "--pdf-password-file".into(),
        password_file.display().to_string(),
    ]);

    assert_eq!(code, 1);
    assert!(!output.exists());

    let batch_dir = directory.join("batch");
    let batch_code = run(&[
        "batch".into(),
        "--out-dir".into(),
        batch_dir.display().to_string(),
        "--pdf-searchable".into(),
    ]);
    assert_eq!(batch_code, 1);
    assert!(!batch_dir.exists());
}

#[test]
fn cli_rejects_literal_password_without_opt_in_and_password_source_conflicts() {
    let unsafe_argv = Cli::try_parse_from([
        "open-scanline",
        "process",
        "--in",
        "input.png",
        "--out",
        "output.pdf",
        "--pdf-password",
        "argv-password",
    ]);
    assert!(unsafe_argv.is_err());

    let explicitly_allowed_argv = Cli::try_parse_from([
        "open-scanline",
        "process",
        "--in",
        "input.png",
        "--out",
        "output.pdf",
        "--pdf-password",
        "argv-password",
        "--allow-insecure-password-argv",
    ]);
    assert!(explicitly_allowed_argv.is_ok());

    let conflicting_sources = Cli::try_parse_from([
        "open-scanline",
        "batch",
        "--out-dir",
        "pages",
        "--pdf-password",
        "argv-password",
        "--allow-insecure-password-argv",
        "--pdf-password-file",
        "password.txt",
    ]);
    assert!(conflicting_sources.is_err());
}

#[test]
fn password_file_invocations_keep_passwords_out_of_the_argv() {
    let invocations = [
        vec![
            "open-scanline",
            "scan",
            "--out",
            "output.pdf",
            "--pdf-password-file",
            "password.txt",
        ],
        vec![
            "open-scanline",
            "process",
            "--in",
            "input.png",
            "--out",
            "output.pdf",
            "--pdf-password-file",
            "password.txt",
        ],
        vec![
            "open-scanline",
            "batch",
            "--out-dir",
            "pages",
            "--pdf-password-file",
            "password.txt",
        ],
    ];

    for argv in invocations {
        let cli = Cli::try_parse_from(argv).unwrap();
        match cli.cmd.unwrap() {
            Commands::Scan {
                pdf_password,
                pdf_password_file,
                ..
            }
            | Commands::Process {
                pdf_password,
                pdf_password_file,
                ..
            }
            | Commands::Batch {
                pdf_password,
                pdf_password_file,
                ..
            } => {
                assert!(pdf_password.is_none());
                assert_eq!(
                    pdf_password_file,
                    Some(std::path::PathBuf::from("password.txt"))
                );
            }
            _ => panic!("expected a PDF export command"),
        }
    }
}

#[test]
fn process_cli_auto_crop_reaches_the_typed_pipeline() {
    let directory = std::env::temp_dir().join("open_scanline_cli_process_auto_crop");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let output = directory.join("output.png");
    let mut data = vec![255_u8; 80 * 40 * 3];
    for y in 15..25 {
        for x in 10..70 {
            let pixel = (y * 80 + x) * 3;
            data[pixel..pixel + 3].fill(20);
        }
    }
    let image = ImageBuffer::new(80, 40, PixelFormat::Rgb8, data).unwrap();
    save_image(&source, &image, None, None).unwrap();

    let code = run(&[
        "process".into(),
        "--in".into(),
        source.display().to_string(),
        "--out".into(),
        output.display().to_string(),
        "--auto-crop".into(),
    ]);

    assert_eq!(code, 0);
    let cropped = load_image(output).unwrap();
    assert_eq!((cropped.width, cropped.height), (64, 14));
}

/// Gating: `scan --width 320 --height 240` with clean config (rotate:0) must
/// write PNG IHDR 320×240 — ambient AppConfig rotate must not pollute when
/// `--config` points at identity defaults.
#[test]
fn scan_cli_clean_config_dimensions_320x240() {
    let dir = std::env::temp_dir().join("open_scanline_cli_clean_scan_dims");
    let _ = fs::create_dir_all(&dir);
    let cfg_path = dir.join("clean_defaults.json");
    let out_a = dir.join("scan_a.png");
    let out_b = dir.join("scan_b.png");
    fs::write(
        &cfg_path,
        r#"{
  "last_device_id": "mock",
  "default_dpi": 150,
  "default_width": 320,
  "default_height": 240,
  "output_dir": "",
  "rotate": 0,
  "flip_h": false,
  "flip_v": false,
  "brightness": 0,
  "contrast": 0,
  "desaturate": false,
  "levels_black": 0,
  "levels_white": 255,
  "levels_gamma": 1.0,
  "invert_colors": false,
  "auto_deskew": false,
  "deskew_angle": 0.0,
  "white_balance": false,
  "language": "en"
}"#,
    )
    .unwrap();

    for out in [&out_a, &out_b] {
        let code = run(&[
            "--config".into(),
            cfg_path.display().to_string(),
            "scan".into(),
            "--device".into(),
            "mock".into(),
            "--out".into(),
            out.display().to_string(),
            "--width".into(),
            "320".into(),
            "--height".into(),
            "240".into(),
        ]);
        assert_eq!(code, 0, "scan exit for {}", out.display());
        assert!(out.is_file());
        let head = fs::read(out).unwrap();
        assert!(
            head.starts_with(b"\x89PNG\r\n\x1a\n"),
            "PNG magic for {}",
            out.display()
        );
        // IHDR width/height at bytes 16..24 (big-endian)
        let w = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
        let h = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
        assert_eq!(w, 320, "IHDR width");
        assert_eq!(h, 240, "IHDR height");
        let img = load_image(out).expect("load_image");
        assert_eq!(img.width, 320);
        assert_eq!(img.height, 240);
    }
}

#[test]
fn scan_and_process_cli_color_controls_reach_the_pipeline() {
    let directory = std::env::temp_dir().join("open_scanline_cli_color_controls");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let processed = directory.join("processed.png");
    let scan_plain = directory.join("scan_plain.png");
    let scan_colored = directory.join("scan_colored.png");
    let image = ImageBuffer::new(
        3,
        1,
        PixelFormat::Rgb8,
        vec![200, 40, 40, 40, 200, 40, 40, 40, 200],
    )
    .unwrap();
    save_image(&source, &image, None, None).unwrap();

    let process_code = run(&[
        "process".into(),
        "--in".into(),
        source.display().to_string(),
        "--out".into(),
        processed.display().to_string(),
        "--saturation".into(),
        "25".into(),
        "--hue".into(),
        "90".into(),
        "--curves".into(),
        "0:0,128:160,255:255".into(),
    ]);
    assert_eq!(process_code, 0);
    assert_ne!(load_image(&processed).unwrap().data, image.data);

    for (out, color_args) in [
        (&scan_plain, Vec::new()),
        (
            &scan_colored,
            vec![
                "--saturation".into(),
                "25".into(),
                "--hue".into(),
                "90".into(),
                "--curves".into(),
                "0:0,128:160,255:255".into(),
            ],
        ),
    ] {
        let mut args = vec![
            "scan".into(),
            "--device".into(),
            "mock".into(),
            "--out".into(),
            out.display().to_string(),
            "--width".into(),
            "32".into(),
            "--height".into(),
            "24".into(),
        ];
        args.extend(color_args);
        assert_eq!(run(&args), 0);
    }
    assert_ne!(
        load_image(&scan_plain).unwrap().data,
        load_image(&scan_colored).unwrap().data
    );
}

#[test]
fn process_loads_config_and_cli_values_override_it() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_process_config_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let configured = directory.join("configured.png");
    let overridden = directory.join("overridden.png");
    let config_path = directory.join("config.json");
    let image = ImageBuffer::new(
        3,
        2,
        PixelFormat::Rgb8,
        vec![
            20, 40, 60, 80, 100, 120, 140, 160, 180, 30, 50, 70, 90, 110, 130, 150, 170, 190,
        ],
    )
    .unwrap();
    save_image(&source, &image, None, None).unwrap();
    save_config(
        &AppConfig {
            rotate: 90,
            brightness: 30,
            ..AppConfig::default()
        },
        &config_path,
    )
    .unwrap();

    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "process".into(),
            "--in".into(),
            source.display().to_string(),
            "--out".into(),
            configured.display().to_string(),
        ]),
        0
    );
    let configured_image = load_image(&configured).unwrap();
    assert_eq!((configured_image.width, configured_image.height), (2, 3));
    assert_ne!(configured_image.data, image.data);

    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "process".into(),
            "--in".into(),
            source.display().to_string(),
            "--out".into(),
            overridden.display().to_string(),
            "--rotate".into(),
            "0".into(),
            "--brightness".into(),
            "0".into(),
        ]),
        0
    );
    let overridden_image = load_image(&overridden).unwrap();
    assert_eq!((overridden_image.width, overridden_image.height), (3, 2));
    assert_eq!(overridden_image.data, image.data);
}

#[test]
fn process_boolean_override_false_beats_config_and_omission_inherits() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_process_boolean_config_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let inherited = directory.join("inherited.png");
    let disabled = directory.join("disabled.png");
    let config_path = directory.join("config.json");
    let image = ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![20, 40, 60, 80, 100, 120]).unwrap();
    save_image(&source, &image, None, None).unwrap();
    save_config(
        &AppConfig {
            invert_colors: true,
            ..AppConfig::default()
        },
        &config_path,
    )
    .unwrap();

    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "process".into(),
            "--in".into(),
            source.display().to_string(),
            "--out".into(),
            inherited.display().to_string(),
        ]),
        0
    );
    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "process".into(),
            "--in".into(),
            source.display().to_string(),
            "--out".into(),
            disabled.display().to_string(),
            "--invert=false".into(),
        ]),
        0
    );

    assert_ne!(load_image(&inherited).unwrap().data, image.data);
    assert_eq!(load_image(&disabled).unwrap().data, image.data);
}

#[test]
fn batch_loads_config_and_cli_values_override_it() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_batch_config_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.json");
    save_config(
        &AppConfig {
            last_device_id: "mock".into(),
            default_width: 12,
            default_height: 8,
            default_dpi: 200,
            batch_pages: 2,
            output_format: "png".into(),
            rotate: 90,
            invert_colors: true,
            ..AppConfig::default()
        },
        &config_path,
    )
    .unwrap();

    let configured = directory.join("configured");
    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "batch".into(),
            "--out-dir".into(),
            configured.display().to_string(),
        ]),
        0
    );
    assert_eq!(
        load_image(configured.join("page_001.png")).unwrap().width,
        8
    );
    assert_eq!(
        load_image(configured.join("page_001.png")).unwrap().height,
        12
    );
    assert!(configured.join("page_002.png").is_file());

    let overridden = directory.join("overridden");
    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "batch".into(),
            "--out-dir".into(),
            overridden.display().to_string(),
            "--pages".into(),
            "1".into(),
            "--width".into(),
            "9".into(),
            "--height".into(),
            "7".into(),
            "--rotate".into(),
            "0".into(),
        ]),
        0
    );
    let image = load_image(overridden.join("page_001.png")).unwrap();
    assert_eq!((image.width, image.height), (9, 7));
    assert!(!overridden.join("page_002.png").exists());
}

#[test]
fn batch_duplex_false_beats_config_when_flatbed_is_requested() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_batch_duplex_config_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.json");
    save_config(
        &AppConfig {
            last_device_id: "mock".into(),
            duplex: true,
            batch_pages: 1,
            ..AppConfig::default()
        },
        &config_path,
    )
    .unwrap();

    let inherited = directory.join("inherited");
    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "batch".into(),
            "--out-dir".into(),
            inherited.display().to_string(),
            "--source".into(),
            "flatbed".into(),
        ]),
        1,
        "omitting the flag must retain configured duplex"
    );
    assert!(!inherited.exists());

    let disabled = directory.join("disabled");
    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "batch".into(),
            "--out-dir".into(),
            disabled.display().to_string(),
            "--source".into(),
            "flatbed".into(),
            "--duplex=false".into(),
        ]),
        0
    );
    assert!(disabled.join("page_001.png").is_file());
}

#[test]
fn malformed_config_does_not_fall_back_or_create_batch_output() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_bad_config_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.json");
    let output = directory.join("batch");
    fs::write(&config_path, "{ definitely not JSON").unwrap();

    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "batch".into(),
            "--out-dir".into(),
            output.display().to_string(),
        ]),
        1
    );
    assert!(!output.exists());
}

#[test]
fn config_update_preserves_an_existing_malformed_file() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_bad_config_update_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    let config_path = directory.join("config.json");
    let sentinel = b"{ definitely not JSON\nkeep this exact content";
    fs::write(&config_path, sentinel).unwrap();

    assert_eq!(
        run(&[
            "--config".into(),
            config_path.display().to_string(),
            "config".into(),
            "--set-dpi".into(),
            "300".into(),
        ]),
        1
    );
    assert_eq!(fs::read(config_path).unwrap(), sentinel);
}

#[test]
fn invalid_explicit_ocr_language_fails_before_scan_output() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_cli_invalid_ocr_language_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&directory);
    let output = directory.join("scan.pdf");

    assert_eq!(
        run(&[
            "scan".into(),
            "--out".into(),
            output.display().to_string(),
            "--pdf-searchable".into(),
            "--ocr-lang".into(),
            "".into(),
        ]),
        1
    );
    assert!(!output.exists());
    assert!(!directory.exists());
}

#[test]
fn cli_rejects_invalid_color_controls_before_output() {
    let directory = std::env::temp_dir().join("open_scanline_cli_invalid_color_controls");
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    let output = directory.join("invalid.png");
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![200, 40, 40]).unwrap();
    save_image(&source, &image, None, None).unwrap();

    let code = run(&[
        "process".into(),
        "--in".into(),
        source.display().to_string(),
        "--out".into(),
        output.display().to_string(),
        "--curves".into(),
        "128:128,0:0".into(),
    ]);
    assert_eq!(code, 2);
    assert!(!output.exists());

    let scan_output = directory.join("invalid_scan.png");
    let scan_code = run(&[
        "scan".into(),
        "--out".into(),
        scan_output.display().to_string(),
        "--saturation".into(),
        "101".into(),
    ]);
    assert_eq!(scan_code, 2);
    assert!(!scan_output.exists());
}
