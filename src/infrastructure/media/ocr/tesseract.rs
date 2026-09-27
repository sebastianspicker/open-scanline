//! Sandboxed system Tesseract adapter.
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::save_image;
use crate::infrastructure::runtime::{
    run_contained_command_with_artifact_quota, ArtifactQuota, ArtifactWatch, CommandSpec,
    TemporaryOutput,
};
use crate::workflows::operation::CancellationToken;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
const MAX_TEXT: u64 = 1024 * 1024;
pub fn available() -> bool {
    which("tesseract").is_some()
}
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    std::env::split_paths(&path).find_map(|dir| {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
        #[cfg(windows)]
        {
            let p = dir.join(format!("{name}.exe"));
            if p.is_file() {
                return Some(p);
            }
        }
        None
    })
}
pub fn recognize(
    image: &ImageBuffer,
    language: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<super::OcrResult> {
    crate::workflows::settings::validate_ocr_language(if language.is_empty() {
        "eng"
    } else {
        language
    })?;
    let bin = which("tesseract").ok_or_else(|| {
        ScanError::Unsupported(
            "Tesseract is not available on PATH; install it or use --offline".into(),
        )
    })?;
    run(image, language, &bin, cancellation)
}
fn run(
    image: &ImageBuffer,
    language: &str,
    bin: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<super::OcrResult> {
    let out = TemporaryOutput::new("ocr", "png")?;
    let base = out.directory().join("output");
    save_image(out.path(), image, None, None)?;
    let input = std::fs::metadata(out.path())?.len();
    let cmd = run_tesseract_command(
        &tesseract_command(bin, out.path(), &base, language),
        out.directory(),
        input,
        bin,
        cancellation,
    )?;
    ensure_tesseract_success(&cmd)?;
    let text = read_tesseract_text(&base.with_extension("txt"))?;
    Ok(tesseract_result(text, language))
}

fn tesseract_command(bin: &Path, input: &Path, base: &Path, language: &str) -> CommandSpec {
    CommandSpec {
        program: bin.display().to_string(),
        args: vec![
            input.display().to_string(),
            base.display().to_string(),
            "-l".into(),
            effective_language(language).into(),
            "--psm".into(),
            "6".into(),
        ],
    }
}

fn effective_language(language: &str) -> &str {
    if language.is_empty() {
        "eng"
    } else {
        language
    }
}

fn run_tesseract_command(
    spec: &CommandSpec,
    directory: &Path,
    input_bytes: u64,
    bin: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<crate::infrastructure::runtime::CommandOutput> {
    run_contained_command_with_artifact_quota(
        spec,
        Duration::from_secs(120),
        &Mutex::new(false),
        cancellation,
        "Tesseract",
        "Tesseract OCR cancelled",
        ArtifactWatch {
            directory,
            quota: ArtifactQuota {
                max_files: 2,
                max_bytes: input_bytes
                    .checked_add(MAX_TEXT)
                    .ok_or_else(|| ScanError::Other("Tesseract artifact quota overflow".into()))?,
            },
        },
    )
    .map_err(|e| match e {
        ScanError::Unsupported(m) if m.contains("failed to start") => {
            ScanError::Unsupported(format!(
                "could not launch Tesseract at {}: {m}; install tesseract or use --offline",
                bin.display()
            ))
        }
        e => e,
    })
}

fn ensure_tesseract_success(cmd: &crate::infrastructure::runtime::CommandOutput) -> Result<()> {
    if cmd.success {
        Ok(())
    } else {
        Err(ScanError::Other(format!(
            "Tesseract failed: {}; check the language data or use --offline",
            String::from_utf8_lossy(&cmd.stderr).trim()
        )))
    }
}

fn read_tesseract_text(path: &Path) -> Result<String> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_TEXT {
        return Err(ScanError::Other(format!(
            "Tesseract text exceeds the {MAX_TEXT} byte limit"
        )));
    }
    let mut b = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(path)?
        .take(MAX_TEXT + 1)
        .read_to_end(&mut b)?;
    if b.len() as u64 > MAX_TEXT {
        return Err(ScanError::Other(format!(
            "Tesseract text exceeds the {MAX_TEXT} byte limit"
        )));
    }
    Ok(String::from_utf8(b)
        .map_err(|e| ScanError::Other(format!("Tesseract text is not UTF-8: {e}")))?
        .trim()
        .to_string())
}

fn tesseract_result(text: String, language: &str) -> super::OcrResult {
    super::OcrResult {
        text: if text.is_empty() {
            "[no text recognized]".into()
        } else {
            text
        },
        engine: "tesseract".into(),
        confidence: 0.8,
        language: effective_language(language).into(),
    }
}
