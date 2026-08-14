//! OCR module (OSL-OCR): offline template heuristic without tesseract.

use crate::backend_process::{
    run_contained_command_with_artifact_quota, ArtifactQuota, ArtifactWatch, CommandSpec,
    TemporaryOutput,
};
#[cfg(test)]
use crate::core::PixelFormat;
use crate::core::{image_to_luma8, ImageBuffer, Result, ScanError};
use crate::device::CancellationToken;
use crate::imaging::load_image;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

pub const OFFLINE_OCR_ENGINE: &str = "offline-template";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    pub engine: String,
    pub confidence: f64,
    pub language: String,
}

impl OcrResult {
    pub fn as_dict(&self) -> Value {
        json!({
            "text": self.text,
            "engine": self.engine,
            "confidence": self.confidence,
            "language": self.language,
            "ok": true,
        })
    }
}

const GRID_WIDTH: usize = 5;
const GRID_HEIGHT: usize = 7;
const GRID_CELLS: usize = GRID_WIDTH * GRID_HEIGHT;

const GLYPH_TEMPLATES: &[(char, &str)] = &[
    ('0', " ### #   ##   ##   ##   ##   # ### "),
    ('1', "  #   ##    #    #    #    #   ### "),
    (
        '2',
        concat!(" ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####"),
    ),
    ('3', " ### #   #    # ###     ##   # ### "),
    (
        '4',
        concat!("   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # "),
    ),
    (
        '5',
        concat!("#####", "#    ", "#### ", "    #", "    #", "#   #", " ### "),
    ),
    ('6', " ### #    #### #   ##   ##   # ### "),
    ('7', "#####    #   #   #   #   #   #   # "),
    ('8', " ### #   ##   # ### #   ##   # ### "),
    ('9', " ### #   ##   # ####    ##   # ### "),
    ('A', " ### #   ##   #######   ##   ##   #"),
    ('B', "#### #   ##   ###### #   ##   #### "),
    ('C', " ### #   ##    #    #    #   # ### "),
    ('E', "######    #    #### #    #    #####"),
    ('F', "######    #    #### #    #    #    "),
    ('H', "#   ##   ##   #######   ##   ##   #"),
    ('I', " ###   #    #    #    #    #   ### "),
    ('K', "#   ##  # # #  ##   # #  #  # #   #"),
    ('L', "#    #    #    #    #    #    #####"),
    ('M', "#   ### ### # ## # ##   ##   ##   #"),
    ('N', "#   ###  ## # ##  ###   ##   ##   #"),
    ('O', " ### #   ##   ##   ##   ##   # ### "),
    ('P', "#### #   ##   ###### #    #    #   "),
    (
        'R',
        concat!("#### ", "#   #", "#   #", "#### ", "# #  ", "#  # ", "#   #"),
    ),
    ('S', " ### #   ##     ###     ##   # ### "),
    ('T', "#####  #    #    #    #    #    #  "),
    ('U', "#   ##   ##   ##   ##   ##   # ### "),
    ('V', "#   ##   ##   ##   ##   ## # #  #  "),
    (
        'W',
        concat!("#   #", "#   #", "#   #", "# # #", "# # #", "## ##", "#   #"),
    ),
    ('X', "#   ##   # # #   #   # # #   ##   #"),
    ('Y', "#   ##   # # #   #    #    #    #  "),
    ('Z', "#####    #   #   #   #   #    #####"),
    (
        '.',
        concat!("     ", "     ", "     ", "     ", "     ", "     ", "  #  "),
    ),
    (
        ',',
        concat!("     ", "     ", "     ", "     ", "     ", "  #  ", " #   "),
    ),
    ('-', "               #####               "),
    (
        ':',
        concat!("     ", "     ", "  #  ", "     ", "  #  ", "     ", "     "),
    ),
    (
        ';',
        concat!("     ", "     ", "  #  ", "     ", "  #  ", " #   ", "     "),
    ),
    ('!', "  #    #    #    #    #         #  "),
];

#[derive(Debug, Clone, Copy)]
struct Component {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Component {
    fn width(self) -> usize {
        self.x1 - self.x0
    }

    fn height(self) -> usize {
        self.y1 - self.y0
    }
}

fn downscale_gray(gray: Vec<u8>, width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    const MAX_WIDTH: usize = 512;
    if width <= MAX_WIDTH {
        return (gray, width, height);
    }
    let scale = MAX_WIDTH as f64 / width as f64;
    let new_height = ((height as f64 * scale).round() as usize).max(1);
    let mut downscaled = vec![0; MAX_WIDTH * new_height];
    for y in 0..new_height {
        let source_y = ((y as f64 / scale) as usize).min(height - 1);
        for x in 0..MAX_WIDTH {
            let source_x = ((x as f64 / scale) as usize).min(width - 1);
            downscaled[y * MAX_WIDTH + x] = gray[source_y * width + source_x];
        }
    }
    (downscaled, MAX_WIDTH, new_height)
}

fn connected_components(binary: &[u8], width: usize, height: usize) -> Vec<Component> {
    let mut visited = vec![false; binary.len()];
    let mut components = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let start = y * width + x;
            if binary[start] == 0 || visited[start] {
                continue;
            }
            components.push(extract_component(
                binary,
                width,
                height,
                start,
                &mut visited,
            ));
        }
    }
    components
}

fn extract_component(
    binary: &[u8],
    width: usize,
    height: usize,
    start: usize,
    visited: &mut [bool],
) -> Component {
    let (mut x0, mut x1, mut y0, mut y1) =
        (start % width, start % width, start / width, start / width);
    let mut stack = vec![start];
    visited[start] = true;
    while let Some(current) = stack.pop() {
        let current_x = current % width;
        let current_y = current / width;
        x0 = x0.min(current_x);
        x1 = x1.max(current_x);
        y0 = y0.min(current_y);
        y1 = y1.max(current_y);
        for next in component_neighbors(current, current_x, current_y, width, height)
            .into_iter()
            .flatten()
        {
            if binary[next] == 0 || visited[next] {
                continue;
            }
            visited[next] = true;
            stack.push(next);
        }
    }
    Component {
        x0,
        y0,
        x1: x1 + 1,
        y1: y1 + 1,
    }
}

fn component_neighbors(
    index: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> [Option<usize>; 4] {
    [
        (x + 1 < width).then(|| index + 1),
        (x > 0).then(|| index - 1),
        (y + 1 < height).then(|| index + width),
        (y > 0).then(|| index - width),
    ]
}

fn match_glyph(binary: &[u8], width: usize, component: Component) -> (char, f64) {
    let grid = glyph_grid(binary, width, component);
    GLYPH_TEMPLATES
        .iter()
        .map(|(glyph, template)| (*glyph, template_confidence(template, grid)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .unwrap_or(('?', 0.0))
}

fn glyph_grid(binary: &[u8], width: usize, component: Component) -> [bool; GRID_CELLS] {
    let mut grid = [false; GRID_CELLS];
    for (index, cell) in grid.iter_mut().enumerate() {
        *cell = cell_has_ink(
            binary,
            width,
            component,
            index % GRID_WIDTH,
            index / GRID_WIDTH,
        );
    }
    grid
}

fn cell_has_ink(
    binary: &[u8],
    width: usize,
    component: Component,
    grid_x: usize,
    grid_y: usize,
) -> bool {
    let y0 = component.y0 + grid_y * component.height() / GRID_HEIGHT;
    let y1 = (component.y0 + ((grid_y + 1) * component.height() / GRID_HEIGHT)).max(y0 + 1);
    let x0 = component.x0 + grid_x * component.width() / GRID_WIDTH;
    let x1 = (component.x0 + ((grid_x + 1) * component.width() / GRID_WIDTH)).max(x0 + 1);
    let ink = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| binary[y * width + x] as usize))
        .sum::<usize>();
    ink * 2 > (y1 - y0) * (x1 - x0)
}

fn template_confidence(template: &str, grid: [bool; GRID_CELLS]) -> f64 {
    let matches = template
        .bytes()
        .zip(grid)
        .filter(|(expected, actual)| (*expected == b'#') == *actual)
        .count();
    matches as f64 / GRID_CELLS as f64
}

fn cluster_lines(mut components: Vec<Component>) -> Vec<Vec<Component>> {
    components.sort_by_key(|component| component.y0);
    let mut lines: Vec<Vec<Component>> = Vec::new();
    for component in components {
        if let Some(line) = lines.iter_mut().find(|line| {
            let top = line
                .iter()
                .map(|item| item.y0)
                .min()
                .unwrap_or(component.y0);
            let bottom = line
                .iter()
                .map(|item| item.y1)
                .max()
                .unwrap_or(component.y1);
            component.y0 <= bottom && component.y1 >= top
        }) {
            line.push(component);
        } else {
            lines.push(vec![component]);
        }
    }
    lines
}

/// Connected-component 5x7 glyph OCR. Engine always `offline-template`.
pub fn ocr_image_offline(image: &ImageBuffer) -> Result<OcrResult> {
    let (w, h, gray) = image_to_luma8(image)?;
    if w == 0 || h == 0 {
        return Ok(offline_result(String::new(), 0.0));
    }

    let (gray, width, height) = downscale_gray(gray, w as usize, h as usize);
    let binary = binarize_gray(gray);
    let components = text_components(&binary, width, height);

    if components.is_empty() {
        return Ok(offline_result("[no text recognized]".into(), 0.0));
    }

    let (text, confidences) = recognize_components(&binary, width, components);
    let text = if text.is_empty() {
        "[no text recognized]".into()
    } else {
        text
    };
    Ok(offline_result(text, average_confidence(&confidences)))
}

fn offline_result(text: String, confidence: f64) -> OcrResult {
    OcrResult {
        text,
        engine: OFFLINE_OCR_ENGINE.into(),
        confidence,
        language: "und".into(),
    }
}

fn binarize_gray(gray: Vec<u8>) -> Vec<u8> {
    gray.into_iter()
        .map(|value| u8::from(value < 128))
        .collect()
}

fn text_components(binary: &[u8], width: usize, height: usize) -> Vec<Component> {
    let max_component_height = (height / 2).max(3);
    let max_component_width = (width * 2 / 5).max(1);
    connected_components(binary, width, height)
        .into_iter()
        .filter(|component| {
            (3..=max_component_height).contains(&component.height())
                && (1..=max_component_width).contains(&component.width())
        })
        .collect()
}

fn recognize_components(
    binary: &[u8],
    width: usize,
    components: Vec<Component>,
) -> (String, Vec<f64>) {
    let mut line_text = Vec::new();
    let mut confidences = Vec::new();
    for line in cluster_lines(components) {
        let (text, line_confidences) = recognize_line(binary, width, line);
        line_text.push(text);
        confidences.extend(line_confidences);
    }
    (line_text.join("\n").trim().to_string(), confidences)
}

fn recognize_line(binary: &[u8], width: usize, mut line: Vec<Component>) -> (String, Vec<f64>) {
    line.sort_by_key(|component| component.x0);
    let gap_threshold = median_component_width(&line) as f64 * 0.8;
    let mut words = Vec::new();
    let mut word = String::new();
    let mut confidences = Vec::new();
    let mut previous_right = None;
    for component in line {
        if previous_right
            .is_some_and(|right| component.x0.saturating_sub(right) as f64 > gap_threshold)
        {
            words.push(std::mem::take(&mut word));
        }
        let (glyph, confidence) = match_glyph(binary, width, component);
        word.push(glyph);
        confidences.push(confidence);
        previous_right = Some(component.x1);
    }
    words.push(word);
    (words.join(" "), confidences)
}

fn median_component_width(line: &[Component]) -> usize {
    let mut widths = line
        .iter()
        .map(|component| component.width())
        .collect::<Vec<_>>();
    widths.sort_unstable();
    widths[widths.len() / 2]
}

fn average_confidence(confidences: &[f64]) -> f64 {
    if confidences.is_empty() {
        0.0
    } else {
        confidences.iter().sum::<f64>() / confidences.len() as f64
    }
}

/// Probe for a system `tesseract` binary.
pub fn tesseract_available() -> bool {
    which_bin("tesseract").is_some()
}

fn which_bin(name: &str) -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let c = dir.join(name);
            if c.is_file() {
                return Some(c);
            }
            #[cfg(windows)]
            {
                let exe = dir.join(format!("{name}.exe"));
                if exe.is_file() {
                    return Some(exe);
                }
            }
        }
    }
    None
}

fn run_tesseract(
    image: &ImageBuffer,
    language: &str,
    bin: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    const TESSERACT_TIMEOUT: Duration = Duration::from_secs(120);
    const MAX_TESSERACT_TEXT_BYTES: u64 = 1024 * 1024;
    let output = TemporaryOutput::new("ocr", "png")?;
    let out_base = output.directory().join("output");
    crate::imaging::save_image(output.path(), image, None, None)?;
    let input_bytes = std::fs::metadata(output.path())?.len();
    let command = run_contained_command_with_artifact_quota(
        &CommandSpec {
            program: bin.display().to_string(),
            args: vec![
                output.path().display().to_string(),
                out_base.display().to_string(),
                "-l".into(),
                if language.is_empty() {
                    "eng".into()
                } else {
                    language.into()
                },
                "--psm".into(),
                "6".into(),
            ],
        },
        TESSERACT_TIMEOUT,
        &Mutex::new(false),
        cancellation,
        "Tesseract",
        "Tesseract OCR cancelled",
        ArtifactWatch {
            directory: output.directory(),
            quota: ArtifactQuota {
                max_files: 2,
                max_bytes: input_bytes
                    .checked_add(MAX_TESSERACT_TEXT_BYTES)
                    .ok_or_else(|| ScanError::Other("Tesseract artifact quota overflow".into()))?,
            },
        },
    )
    .map_err(|error| match error {
        ScanError::Unsupported(message) if message.contains("failed to start") => {
            ScanError::Unsupported(format!(
                "could not launch Tesseract at {}: {message}; install tesseract or use --offline",
                bin.display()
            ))
        }
        other => other,
    })?;
    if !command.success {
        return Err(ScanError::Other(format!(
            "Tesseract failed: {}; check the language data or use --offline",
            String::from_utf8_lossy(&command.stderr).trim()
        )));
    }
    let text_path = out_base.with_extension("txt");
    let metadata = std::fs::metadata(&text_path)?;
    if metadata.len() > MAX_TESSERACT_TEXT_BYTES {
        return Err(ScanError::Other(format!(
            "Tesseract text exceeds the {MAX_TESSERACT_TEXT_BYTES} byte limit"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(&text_path)?
        .take(MAX_TESSERACT_TEXT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_TESSERACT_TEXT_BYTES {
        return Err(ScanError::Other(format!(
            "Tesseract text exceeds the {MAX_TESSERACT_TEXT_BYTES} byte limit"
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|error| ScanError::Other(format!("Tesseract text is not UTF-8: {error}")))?
        .trim()
        .to_string();
    Ok(OcrResult {
        text: if text.is_empty() {
            "[no text recognized]".into()
        } else {
            text
        },
        engine: "tesseract".into(),
        confidence: 0.8,
        language: if language.is_empty() {
            "eng".into()
        } else {
            language.into()
        },
    })
}

/// Run Tesseract OCR. Missing executables and failed invocations are explicit;
/// callers select the built-in recognizer with `offline = true`.
pub fn ocr_image_tesseract(image: &ImageBuffer, language: &str) -> Result<OcrResult> {
    ocr_image_tesseract_with_cancellation(image, language, None)
}

/// Run Tesseract while allowing a scan or batch token to terminate it.
pub fn ocr_image_tesseract_with_cancellation(
    image: &ImageBuffer,
    language: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    crate::config::validate_ocr_language(if language.is_empty() { "eng" } else { language })?;
    let Some(bin) = which_bin("tesseract") else {
        return Err(ScanError::Unsupported(
            "Tesseract is not available on PATH; install it or use --offline".into(),
        ));
    };
    run_tesseract(image, language, &bin, cancellation)
}

pub fn ocr_image(image: &ImageBuffer, language: &str, offline: bool) -> Result<OcrResult> {
    ocr_image_with_cancellation(image, language, offline, None)
}

/// Run OCR while forwarding cancellation to command-backed engines.
pub fn ocr_image_with_cancellation(
    image: &ImageBuffer,
    language: &str,
    offline: bool,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    if offline {
        return ocr_image_offline(image);
    }
    ocr_image_tesseract_with_cancellation(image, language, cancellation)
}

pub fn ocr_file(path: impl AsRef<Path>, language: &str, offline: bool) -> Result<OcrResult> {
    let img = load_image(path)?;
    ocr_image(&img, language, offline)
}

/// Run OCR for a file while forwarding cancellation to command-backed engines.
pub fn ocr_file_with_cancellation(
    path: impl AsRef<Path>,
    language: &str,
    offline: bool,
    cancellation: CancellationToken,
) -> Result<OcrResult> {
    let img = load_image(path)?;
    ocr_image_with_cancellation(&img, language, offline, Some(&cancellation))
}

pub fn ocr_module_info() -> Value {
    let tess = tesseract_available();
    json!({
        "tesseract_available": tess,
        "engines": if tess {
            vec![OFFLINE_OCR_ENGINE, "tesseract"]
        } else {
            vec![OFFLINE_OCR_ENGINE]
        },
        "default_language": "eng",
        "ok": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: [&str; 7] = [
        "#   #", "#   #", "#   #", "#####", "#   #", "#   #", "#   #",
    ];

    struct TestCanvas<'a> {
        pixels: &'a mut [u8],
        width: usize,
    }

    impl TestCanvas<'_> {
        fn paint_block(&mut self, x: usize, y: usize) {
            for dy in 0..3 {
                for dx in 0..3 {
                    self.pixels[(y + dy) * self.width + x + dx] = 0;
                }
            }
        }
    }

    fn paint_glyph(canvas: &mut TestCanvas<'_>, x: usize, y: usize, rows: &[&str]) {
        for (row, pixels) in rows.iter().enumerate() {
            for (column, pixel) in pixels.bytes().enumerate() {
                if pixel != b'#' {
                    continue;
                }
                canvas.paint_block(x + column * 3, y + row * 3);
            }
        }
    }

    #[test]
    fn offline_ocr_recognizes_5x7_glyphs_and_word_gaps() {
        assert!(GLYPH_TEMPLATES
            .iter()
            .all(|(_, template)| template.len() == GRID_CELLS));
        let (width, height) = (80, 60);
        let mut data = vec![255; width * height];
        let mut canvas = TestCanvas {
            pixels: &mut data,
            width,
        };
        paint_glyph(&mut canvas, 10, 10, &H);
        paint_glyph(&mut canvas, 40, 10, &H);
        let image =
            ImageBuffer::new(width as u32, height as u32, PixelFormat::Gray8, data).unwrap();
        let result = ocr_image_offline(&image).unwrap();
        assert_eq!(result.text, "H H");
        assert!(result.confidence > 0.99);
    }

    #[test]
    fn offline_ocr_marks_images_without_components() {
        let image = ImageBuffer::new(8, 8, PixelFormat::Gray8, vec![255; 64]).unwrap();
        let result = ocr_image_offline(&image).unwrap();
        assert_eq!(result.text, "[no text recognized]");
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn offline_ocr_preserves_line_order() {
        let (width, height) = (40, 80);
        let mut data = vec![255; width * height];
        let mut canvas = TestCanvas {
            pixels: &mut data,
            width,
        };
        paint_glyph(&mut canvas, 10, 5, &H);
        paint_glyph(&mut canvas, 10, 45, &H);
        let image =
            ImageBuffer::new(width as u32, height as u32, PixelFormat::Gray8, data).unwrap();
        let result = ocr_image_offline(&image).unwrap();
        assert_eq!(result.text, "H\nH");
        assert!(result.confidence > 0.99);
    }

    #[test]
    fn tesseract_launch_failure_is_explicit() {
        let image = ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![255]).unwrap();
        let error = run_tesseract(
            &image,
            "eng",
            Path::new("/definitely/not/an/open-scanline-tesseract"),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("use --offline"));
    }

    #[cfg(unix)]
    fn fake_tesseract_script(name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "open-scanline-{name}-{}-{}.sh",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_cancellation_kills_fake_hanging_executable() {
        let script = fake_tesseract_script("tesseract-cancel", "sleep 30");
        let token = CancellationToken::new();
        let trigger = token.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            trigger.cancel();
        });
        let started = std::time::Instant::now();
        let error = run_tesseract(
            &ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![255]).unwrap(),
            "eng",
            &script,
            Some(&token),
        )
        .unwrap_err();
        canceller.join().unwrap();
        std::fs::remove_file(script).unwrap();

        assert!(
            matches!(error, ScanError::Cancelled(message) if message == "Tesseract OCR cancelled")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_rejects_oversized_fake_output() {
        let script = fake_tesseract_script(
            "tesseract-oversized",
            "dd if=/dev/zero of=\"$2.txt\" bs=1048577 count=1 2>/dev/null",
        );
        let error = run_tesseract(
            &ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![255]).unwrap(),
            "eng",
            &script,
            None,
        )
        .unwrap_err();
        std::fs::remove_file(script).unwrap();
        assert!(
            error.to_string().contains("byte limit")
                || error.to_string().contains("artifact output exceeded"),
            "unexpected oversized-output error: {error}"
        );
    }
}
