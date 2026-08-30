use super::{cancellation, commands, info, process, scan};
use crate::inbound::cli::args::{Commands, OcrEngineArg};
use crate::infrastructure::config::json::load_config;
use crate::workflows::settings::AppConfig;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const MAX_PDF_PASSWORD_BYTES: usize = 4096;

pub(super) fn dispatch_command(command: Commands, config_path: Option<&Path>) -> i32 {
    match command {
        Commands::Scan {
            device,
            allow_unlisted_escl,
            out,
            pdf_searchable,
            pdf_password,
            pdf_password_file,
            allow_insecure_password_argv,
            ocr_lang,
            ocr_engine,
            scanner_profile,
            width,
            height,
            seed,
            dpi,
            source,
            duplex,
            rotate,
            flip_h,
            flip_v,
            crop,
            brightness,
            contrast,
            saturation,
            hue,
            curves,
            desaturate,
            levels_black,
            levels_white,
            levels_gamma,
            auto_deskew,
            deskew,
            white_balance,
            auto_levels,
            auto_crop,
            auto_orient,
            infrared_clean,
            descreen,
            descreen_dpi,
            sharpen,
            film_type,
            invert,
            restore_colors,
            restore_fading,
            grain_reduction,
            flatten,
            hole_punch,
            colorize_mode,
            raw_out,
        } => {
            let config = match load_runtime_config(config_path) {
                Ok(config) => config,
                Err(()) => return 1,
            };
            let export = match build_export_options(
                &config,
                ExportOptionOverrides {
                    pdf_searchable,
                    pdf_password,
                    pdf_password_file,
                    allow_insecure_password_argv,
                    ocr_language: ocr_lang,
                    ocr_engine,
                    scanner_profile,
                },
            ) {
                Ok(export) => export,
                Err(error) => return report_export_option_error(error),
            };
            cancellation::with_registered_token(move |cancellation| {
                scan::run(
                    scan::Request {
                        config,
                        export,
                        acquisition: scan::Acquisition {
                            device,
                            allow_unlisted_escl,
                            out,
                            width,
                            height,
                            seed,
                            dpi,
                            source,
                            duplex,
                            invert,
                            raw_out,
                        },
                        overrides: scan::PipelineOverrides {
                            geometry: scan::GeometryOverrides {
                                rotate,
                                flip_h,
                                flip_v,
                                crop,
                                auto_deskew,
                                deskew,
                            },
                            color: scan::ColorOverrides {
                                brightness,
                                contrast,
                                saturation,
                                hue,
                                curves: curves.map(|curves| curves.0),
                                desaturate,
                                levels_black,
                                levels_white,
                                levels_gamma,
                                white_balance,
                                auto_levels,
                            },
                            filters: scan::FilterOverrides {
                                infrared_clean,
                                descreen,
                                descreen_dpi,
                                sharpen,
                                film_type,
                                restore_colors,
                                restore_fading,
                                grain_reduction,
                                flatten,
                                hole_punch,
                                colorize_mode,
                            },
                        },
                        auto_crop,
                        auto_orient,
                    },
                    cancellation,
                )
            })
        }
        Commands::Devices => commands::devices(),
        Commands::Manufacturers { json, resolve } => commands::manufacturers(json, resolve),
        Commands::Config {
            init,
            show,
            set_output_dir,
            set_dpi,
            set_device,
        } => commands::config(commands::ConfigRequest {
            config_path,
            init,
            show,
            set_output_dir,
            set_dpi,
            set_device,
        }),
        Commands::Gui => commands::gui(config_path),
        Commands::Plugin { out, device, quiet } => {
            cancellation::with_registered_token(move |cancellation| {
                commands::plugin(config_path, out, device, quiet, cancellation)
            })
        }
        Commands::Convert { inp, out, dpi } => {
            cancellation::with_registered_token(move |cancellation| {
                commands::convert(inp, out, dpi, cancellation)
            })
        }
        Commands::Ocr { inp, lang, offline } => {
            cancellation::with_registered_token(move |cancellation| {
                commands::ocr(inp, lang, offline, cancellation)
            })
        }
        Commands::Onnx {
            inp,
            model,
            input_name,
            layout,
            normalization,
        } => commands::onnx(inp, model, input_name, layout, normalization),
        Commands::OnnxWorker {
            inp,
            model,
            report,
            input_name,
            layout,
            normalization,
            worker_protocol,
        } => commands::onnx_worker(
            inp,
            model,
            report,
            input_name,
            layout,
            normalization,
            worker_protocol,
        ),
        Commands::Process {
            inp,
            out,
            pdf_searchable,
            pdf_password,
            pdf_password_file,
            allow_insecure_password_argv,
            ocr_lang,
            ocr_engine,
            scanner_profile,
            rotate,
            flip_h,
            flip_v,
            crop,
            brightness,
            contrast,
            saturation,
            hue,
            curves,
            desaturate,
            levels_black,
            levels_white,
            levels_gamma,
            invert,
            sharpen,
            auto_levels,
            auto_crop,
            auto_orient,
            auto_deskew,
            deskew,
            white_balance,
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
            quality,
        } => {
            let config = match load_runtime_config(config_path) {
                Ok(config) => config,
                Err(()) => return 1,
            };
            let export = match build_export_options(
                &config,
                ExportOptionOverrides {
                    pdf_searchable,
                    pdf_password,
                    pdf_password_file,
                    allow_insecure_password_argv,
                    ocr_language: ocr_lang,
                    ocr_engine,
                    scanner_profile,
                },
            ) {
                Ok(export) => export,
                Err(error) => return report_export_option_error(error),
            };
            cancellation::with_registered_token(move |cancellation| {
                process::run(
                    process::Request {
                        config,
                        inp,
                        out,
                        export,
                        overrides: scan::PipelineOverrides {
                            geometry: scan::GeometryOverrides {
                                rotate,
                                flip_h,
                                flip_v,
                                crop,
                                auto_deskew,
                                deskew,
                            },
                            color: scan::ColorOverrides {
                                brightness,
                                contrast,
                                saturation,
                                hue,
                                curves: curves.map(|curves| curves.0),
                                desaturate,
                                levels_black,
                                levels_white,
                                levels_gamma,
                                white_balance,
                                auto_levels,
                            },
                            filters: scan::FilterOverrides {
                                infrared_clean,
                                descreen,
                                descreen_dpi,
                                sharpen,
                                film_type,
                                restore_colors,
                                restore_fading,
                                grain_reduction,
                                flatten,
                                hole_punch,
                                colorize_mode,
                            },
                        },
                        invert,
                        auto_crop,
                        auto_orient,
                        quality,
                    },
                    cancellation,
                )
            })
        }
        Commands::Batch {
            device,
            allow_unlisted_escl,
            out_dir,
            pages,
            width,
            height,
            seed,
            dpi,
            source,
            duplex,
            format,
            rotate,
            flip_h,
            flip_v,
            crop,
            brightness,
            contrast,
            saturation,
            hue,
            curves,
            desaturate,
            levels_black,
            levels_white,
            levels_gamma,
            auto_deskew,
            deskew,
            auto_crop,
            auto_orient,
            white_balance,
            auto_levels,
            infrared_clean,
            descreen,
            descreen_dpi,
            sharpen,
            film_type,
            invert,
            restore_colors,
            restore_fading,
            grain_reduction,
            flatten,
            hole_punch,
            colorize_mode,
            multipage_tiff,
            multipage_pdf,
            multipage_out,
            pdf_searchable,
            pdf_password,
            pdf_password_file,
            allow_insecure_password_argv,
            ocr_lang,
            ocr_engine,
            scanner_profile,
            contact_sheet,
        } => {
            let config = match load_runtime_config(config_path) {
                Ok(config) => config,
                Err(()) => return 1,
            };
            let export = match build_export_options(
                &config,
                ExportOptionOverrides {
                    pdf_searchable,
                    pdf_password,
                    pdf_password_file,
                    allow_insecure_password_argv,
                    ocr_language: ocr_lang,
                    ocr_engine,
                    scanner_profile,
                },
            ) {
                Ok(export) => export,
                Err(error) => return report_export_option_error(error),
            };
            cancellation::with_registered_token(move |cancellation| {
                commands::batch(
                    commands::BatchRequest {
                        config,
                        device,
                        allow_unlisted_escl,
                        out_dir,
                        pages,
                        width,
                        height,
                        seed,
                        dpi,
                        source,
                        duplex,
                        format,
                        overrides: scan::PipelineOverrides {
                            geometry: scan::GeometryOverrides {
                                rotate,
                                flip_h,
                                flip_v,
                                crop,
                                auto_deskew,
                                deskew,
                            },
                            color: scan::ColorOverrides {
                                brightness,
                                contrast,
                                saturation,
                                hue,
                                curves: curves.map(|curves| curves.0),
                                desaturate,
                                levels_black,
                                levels_white,
                                levels_gamma,
                                white_balance,
                                auto_levels,
                            },
                            filters: scan::FilterOverrides {
                                infrared_clean,
                                descreen,
                                descreen_dpi,
                                sharpen,
                                film_type,
                                restore_colors,
                                restore_fading,
                                grain_reduction,
                                flatten,
                                hole_punch,
                                colorize_mode,
                            },
                        },
                        auto_crop,
                        auto_orient,
                        invert,
                        multipage_tiff,
                        multipage_pdf,
                        multipage_out,
                        export,
                        contact_sheet,
                    },
                    cancellation,
                )
            })
        }
        Commands::Package { binary, out } => commands::package(binary, out),
        Commands::HelpText => commands::help_text(),
        Commands::Info { module } => info::run(module),
    }
}

fn report_export_option_error(error: String) -> i32 {
    eprintln!("PDF password error: {error}");
    1
}

fn load_runtime_config(config_path: Option<&Path>) -> Result<AppConfig, ()> {
    load_config(config_path).map_err(|error| {
        eprintln!("config error: {error}");
    })
}

#[derive(Default)]
struct ExportOptionOverrides {
    pdf_searchable: bool,
    pdf_password: Option<String>,
    pdf_password_file: Option<PathBuf>,
    allow_insecure_password_argv: bool,
    ocr_language: Option<String>,
    ocr_engine: Option<OcrEngineArg>,
    scanner_profile: Option<PathBuf>,
}

/// Construct export controls before a scan, process, or batch handler can mutate output.
fn build_export_options(
    config: &AppConfig,
    overrides: ExportOptionOverrides,
) -> Result<crate::ExportOptions, String> {
    let ExportOptionOverrides {
        pdf_searchable,
        pdf_password,
        pdf_password_file,
        allow_insecure_password_argv,
        ocr_language,
        ocr_engine,
        scanner_profile,
    } = overrides;
    let pdf_password = resolve_pdf_password(
        pdf_password,
        pdf_password_file.as_deref(),
        allow_insecure_password_argv,
    )?;
    let ocr_language = ocr_language.unwrap_or_else(|| config.ocr_language.clone());
    crate::workflows::settings::validate_ocr_language(&ocr_language)
        .map_err(|error| error.to_string())?;
    Ok(crate::ExportOptions {
        pdf_password,
        searchable_pdf: pdf_searchable,
        ocr_language,
        ocr_engine: match ocr_engine {
            Some(engine) => engine.as_export(),
            None => ocr_engine_from_config(&config.ocr_engine)?,
        },
        scanner_profile,
    })
}

fn ocr_engine_from_config(value: &str) -> Result<crate::OcrEngine, String> {
    match value.to_ascii_lowercase().as_str() {
        "offline" => Ok(crate::OcrEngine::Offline),
        "tesseract" => Ok(crate::OcrEngine::Tesseract),
        _ => Err("config OCR engine must be offline or tesseract".into()),
    }
}

fn resolve_pdf_password(
    mut pdf_password: Option<String>,
    pdf_password_file: Option<&Path>,
    allow_insecure_password_argv: bool,
) -> Result<Option<String>, String> {
    if pdf_password.is_some() && pdf_password_file.is_some() {
        clear_pdf_password(&mut pdf_password);
        return Err("--pdf-password conflicts with --pdf-password-file".into());
    }
    if pdf_password.is_some() && !allow_insecure_password_argv {
        clear_pdf_password(&mut pdf_password);
        return Err("--pdf-password requires --allow-insecure-password-argv".into());
    }
    match pdf_password_file {
        Some(path) => read_pdf_password(path).map(Some),
        None => Ok(pdf_password),
    }
}

fn read_pdf_password(path: &Path) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(MAX_PDF_PASSWORD_BYTES + 1);
    let result = if path == Path::new("-") {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        read_password_bytes(&mut stdin, &mut bytes)
    } else {
        let mut file = File::open(path)
            .map_err(|error| format!("could not open password file {}: {error}", path.display()))?;
        read_password_bytes(&mut file, &mut bytes)
    };
    if let Err(error) = result {
        bytes.fill(0);
        return Err(error);
    }
    password_from_bytes(&mut bytes)
}

fn read_password_bytes(reader: &mut dyn Read, bytes: &mut Vec<u8>) -> Result<(), String> {
    reader
        .take((MAX_PDF_PASSWORD_BYTES + 1) as u64)
        .read_to_end(bytes)
        .map_err(|error| format!("could not read password input: {error}"))?;
    if bytes.len() > MAX_PDF_PASSWORD_BYTES {
        return Err(format!(
            "password input exceeds the {} byte limit",
            MAX_PDF_PASSWORD_BYTES
        ));
    }
    Ok(())
}

fn password_from_bytes(bytes: &mut Vec<u8>) -> Result<String, String> {
    trim_one_line_ending(bytes);
    let result = if bytes.is_empty() {
        Err("password input must not be empty".into())
    } else if bytes.contains(&0) {
        Err("password input must not contain NUL bytes".into())
    } else {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "password input must be valid UTF-8".into())
    };
    bytes.fill(0);
    bytes.clear();
    result
}

fn trim_one_line_ending(bytes: &mut Vec<u8>) {
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if matches!(bytes.last(), Some(b'\r' | b'\n')) {
        bytes.truncate(bytes.len() - 1);
    }
}

/// Overwrite a handler-held PDF password as soon as routing completes.
pub(super) fn clear_pdf_password(pdf_password: &mut Option<String>) {
    if let Some(password) = pdf_password {
        password.replace_range(.., &"\0".repeat(password.len()));
        password.clear();
    }
    *pdf_password = None;
}
