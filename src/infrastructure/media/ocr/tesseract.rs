//! Sandboxed system Tesseract adapter.
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::save_image;
use crate::infrastructure::runtime::{
    run_contained_command_with_artifact_quota, ArtifactQuota, ArtifactWatch, CommandSpec,
    TemporaryOutput,
};
use crate::operation::CancellationToken;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
const MAX_TEXT: u64 = 1024 * 1024;
/// Remedy shown whenever Tesseract cannot be used. `--offline` exists only on
/// the deprecated `ocr` path, so name the real options of each command.
const ENGINE_HINT: &str =
    "use --ocr-engine offline|ocrs (scan/process/batch) or --engine offline|ocrs (ocr)";
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
    crate::domain::settings::validate_ocr_language(if language.is_empty() {
        "eng"
    } else {
        language
    })?;
    let bin = which("tesseract").ok_or_else(|| {
        ScanError::Unsupported(format!(
            "Tesseract is not available on PATH; install it or {ENGINE_HINT}"
        ))
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
    save_image(out.path(), image, None, None)?;
    let input = std::fs::metadata(out.path())?.len();
    // Leptonica rewrites paths that start with `/tmp` on macOS, so a temp
    // directory reached through the `/tmp` symlink would not be found. Pass
    // the resolved path (`/private/tmp/...`) instead.
    let directory = std::fs::canonicalize(out.directory())?;
    let base = directory.join("output");
    let input_path = directory.join(out.path().file_name().unwrap_or_default());
    let cmd = run_tesseract_command(
        &tesseract_command(bin, &input_path, &base, language),
        out.directory(),
        input,
        bin,
        cancellation,
    )?;
    ensure_tesseract_success(&cmd)?;
    let text = read_tesseract_text(&base.with_extension("txt"))?;
    // Confidence is best effort: a build that skips the TSV still yields text.
    let confidence = read_tesseract_output(&base.with_extension("tsv"))
        .map(|tsv| tsv_confidence(&tsv))
        .unwrap_or(0.0);
    Ok(tesseract_result(text, confidence, language))
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
            // One pass writes both the plain text and the per-word TSV that
            // the confidence is computed from.
            "txt".into(),
            "tsv".into(),
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
                max_files: 3,
                max_bytes: input_bytes
                    .checked_add(2 * MAX_TEXT)
                    .ok_or_else(|| ScanError::Other("Tesseract artifact quota overflow".into()))?,
            },
        },
    )
    .map_err(|e| match e {
        ScanError::Unsupported(m) if m.contains("failed to start") => {
            ScanError::Unsupported(format!(
                "could not launch Tesseract at {}: {m}; install tesseract or {ENGINE_HINT}",
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
            "Tesseract failed: {}; check the language data or {ENGINE_HINT}",
            String::from_utf8_lossy(&cmd.stderr).trim()
        )))
    }
}

fn read_tesseract_text(path: &Path) -> Result<String> {
    Ok(read_tesseract_output(path)?.trim().to_string())
}

fn read_tesseract_output(path: &Path) -> Result<String> {
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
    String::from_utf8(b).map_err(|e| ScanError::Other(format!("Tesseract text is not UTF-8: {e}")))
}

/// Mean word confidence (0.0..=1.0) from Tesseract TSV output. Only word rows
/// (level 5) with a non-negative `conf` and visible text count; Tesseract
/// reports -1 for layout rows. No qualifying word gives 0.0.
fn tsv_confidence(tsv: &str) -> f64 {
    const LEVEL: usize = 0;
    const CONF: usize = 10;
    const TEXT: usize = 11;
    let mut sum = 0.0;
    let mut words = 0_u32;
    for row in tsv.lines().skip(1) {
        let columns = row.split('\t').collect::<Vec<_>>();
        if columns.len() <= TEXT || columns[LEVEL] != "5" || columns[TEXT].trim().is_empty() {
            continue;
        }
        if let Ok(conf) = columns[CONF].trim().parse::<f64>() {
            if conf >= 0.0 {
                sum += conf;
                words += 1;
            }
        }
    }
    if words == 0 {
        0.0
    } else {
        (sum / f64::from(words) / 100.0).clamp(0.0, 1.0)
    }
}

fn tesseract_result(text: String, confidence: f64, language: &str) -> super::OcrResult {
    super::OcrResult {
        text: if text.is_empty() {
            super::NO_TEXT_PLACEHOLDER.into()
        } else {
            text
        },
        engine: "tesseract".into(),
        confidence,
        language: effective_language(language).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from `tesseract page.png out --psm 6 tsv` (Tesseract 5.x).
    const SAMPLE_TSV: &str = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
1\t1\t0\t0\t0\t0\t0\t0\t640\t200\t-1\t\n\
2\t1\t1\t0\t0\t0\t20\t20\t600\t60\t-1\t\n\
3\t1\t1\t1\t0\t0\t20\t20\t600\t60\t-1\t\n\
4\t1\t1\t1\t1\t0\t20\t20\t600\t60\t-1\t\n\
5\t1\t1\t1\t1\t1\t20\t20\t180\t60\t96.123\tHello\n\
5\t1\t1\t1\t1\t2\t220\t20\t200\t60\t88.5\tscanner\n\
5\t1\t1\t1\t1\t3\t440\t20\t180\t60\t0\t.\n\
5\t1\t1\t1\t1\t4\t640\t20\t10\t60\t-1\tghost\n\
5\t1\t1\t1\t1\t5\t660\t20\t10\t60\t99\t \n";

    #[test]
    fn tsv_confidence_averages_non_negative_words() {
        let expected = (96.123 + 88.5 + 0.0) / 3.0 / 100.0;
        assert!((tsv_confidence(SAMPLE_TSV) - expected).abs() < 1e-9);
    }

    #[test]
    fn tsv_confidence_is_zero_without_words() {
        let header = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n";
        assert_eq!(tsv_confidence(""), 0.0);
        assert_eq!(tsv_confidence(header), 0.0);
        assert_eq!(
            tsv_confidence(&format!("{header}1\t1\t0\t0\t0\t0\t0\t0\t9\t9\t-1\t\n")),
            0.0
        );
    }

    #[test]
    fn hints_name_the_real_engine_options() {
        assert!(ENGINE_HINT.contains("--ocr-engine"));
        assert!(ENGINE_HINT.contains("--engine"));
        assert!(!ENGINE_HINT.contains("--offline"));
    }
}
