use crate::inbound::cli::args::ExportOptions;
use crate::workflows::settings::AppConfig;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const MAX_PDF_PASSWORD_BYTES: usize = 4096;

pub(super) fn build(
    config: &AppConfig,
    overrides: ExportOptions,
) -> Result<crate::ExportOptions, String> {
    let ExportOptions {
        pdf_searchable,
        pdf_password,
        pdf_password_file,
        allow_insecure_password_argv,
        ocr_lang,
        ocr_engine,
        scanner_profile,
    } = overrides;
    let pdf_password = resolve_pdf_password(
        pdf_password,
        pdf_password_file.as_deref(),
        allow_insecure_password_argv,
    )?;
    let ocr_language = ocr_lang.unwrap_or_else(|| config.ocr_language.clone());
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

pub(super) fn report_error(error: String) -> i32 {
    eprintln!("PDF password error: {error}");
    1
}

fn ocr_engine_from_config(value: &str) -> Result<crate::OcrEngine, String> {
    match value.to_ascii_lowercase().as_str() {
        "offline" => Ok(crate::OcrEngine::Offline),
        "ocrs" => Ok(crate::OcrEngine::Ocrs),
        "tesseract" => Ok(crate::OcrEngine::Tesseract),
        _ => Err("config OCR engine must be offline, ocrs, or tesseract".into()),
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
        read_password_bytes(&mut stdin.lock(), &mut bytes)
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
