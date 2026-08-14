//! ML helpers (OSL-ML): auto-crop and orientation — pure image statistics.

use crate::core::{image_to_luma8, ImageBuffer, Rect, Result, Rotate};
use crate::pipeline::{crop, rotate};
use serde_json::{json, Value};

#[path = "ml/isolation.rs"]
mod isolation;
#[path = "ml/onnx.rs"]
mod onnx;

pub(crate) use isolation::{run_isolated_onnx, run_onnx_worker};
pub use onnx::{
    run_user_onnx, run_user_onnx_with_options, run_user_onnx_with_worker, OnnxInferenceOptions,
    OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
};

#[derive(Debug, Clone)]
pub struct AutoCropResult {
    pub rect: Rect,
    pub confidence: f64,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct OrientationResult {
    pub degrees: i32,
    pub confidence: f64,
    pub method: String,
}

/// Content-aware bounding box (threshold default 240).
pub fn auto_crop_bounds(
    image: &ImageBuffer,
    threshold: i32,
    margin: i32,
) -> Result<AutoCropResult> {
    let (w, h, gray) = image_to_luma8(image)?;
    let thr = threshold.clamp(0, 255) as u8;
    let mut min_x = w as i32;
    let mut min_y = h as i32;
    let mut max_x = -1i32;
    let mut max_y = -1i32;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let g = gray[(y as u32 * w + x as u32) as usize];
            if g < thr {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if max_x < 0 {
        return Ok(AutoCropResult {
            rect: Rect::new(0, 0, w, h),
            confidence: 0.0,
            method: "full-frame-empty".into(),
        });
    }
    min_x = (min_x - margin).max(0);
    min_y = (min_y - margin).max(0);
    max_x = (max_x + margin).min(w as i32 - 1);
    max_y = (max_y + margin).min(h as i32 - 1);
    let rw = (max_x - min_x + 1).max(1) as u32;
    let rh = (max_y - min_y + 1).max(1) as u32;
    let area_ratio = (rw * rh) as f64 / ((w * h).max(1) as f64);
    let conf = (1.0 - area_ratio + 0.2).clamp(0.0, 1.0);
    Ok(AutoCropResult {
        rect: Rect::new(min_x, min_y, rw, rh),
        confidence: conf,
        method: "luma-threshold-bbox".into(),
    })
}

/// Apply auto-crop at threshold 240 (product default).
pub fn apply_auto_crop(image: &ImageBuffer) -> Result<ImageBuffer> {
    apply_auto_crop_with_params(image, 240, 2)
}

pub fn apply_auto_crop_with_params(
    image: &ImageBuffer,
    threshold: i32,
    margin: i32,
) -> Result<ImageBuffer> {
    let result = auto_crop_bounds(image, threshold, margin)?;
    if result.rect.width == image.width && result.rect.height == image.height {
        return Ok(image.clone());
    }
    crop(image, result.rect)
}

pub fn detect_orientation(image: &ImageBuffer) -> Result<OrientationResult> {
    let (w, h, gray) = image_to_luma8(image)?;
    if w < 3 || h < 3 {
        return Ok(OrientationResult {
            degrees: 0,
            confidence: 0.0,
            method: "too-small".into(),
        });
    }
    let mut gx_sum: u64 = 0;
    let mut gy_sum: u64 = 0;
    for y in 1..(h as usize - 1) {
        for x in 1..(w as usize - 1) {
            let i = y * w as usize + x;
            let gx = (gray[i + 1] as i32 - gray[i - 1] as i32).unsigned_abs() as u64;
            let gy =
                (gray[i + w as usize] as i32 - gray[i - w as usize] as i32).unsigned_abs() as u64;
            gx_sum += gx;
            gy_sum += gy;
        }
    }
    let total = gx_sum + gy_sum + 1;
    let horiz = gx_sum as f64 / total as f64;
    let vert = gy_sum as f64 / total as f64;
    let (mut degrees, mut conf) = if horiz >= vert {
        (0, horiz)
    } else {
        (90, vert)
    };
    if degrees == 0 {
        let top: f64 = gray[0..w as usize].iter().map(|&v| v as f64).sum::<f64>() / w as f64;
        let bot_start = ((h - 1) * w) as usize;
        let bot: f64 = gray[bot_start..bot_start + w as usize]
            .iter()
            .map(|&v| v as f64)
            .sum::<f64>()
            / w as f64;
        if top < bot - 8.0 {
            degrees = 180;
            conf = (conf + 0.1).min(1.0);
        }
    }
    Ok(OrientationResult {
        degrees,
        confidence: conf.min(1.0),
        method: "gradient-energy".into(),
    })
}

pub fn apply_orientation(image: &ImageBuffer) -> Result<ImageBuffer> {
    apply_orientation_with_degrees(image, None)
}

pub fn apply_orientation_with_degrees(
    image: &ImageBuffer,
    degrees: Option<i32>,
) -> Result<ImageBuffer> {
    let deg = match degrees {
        Some(d) => d.rem_euclid(360),
        None => detect_orientation(image)?.degrees.rem_euclid(360),
    };
    match deg {
        0 => Ok(image.clone()),
        90 => rotate(image, Rotate::R90),
        180 => rotate(image, Rotate::R180),
        270 => rotate(image, Rotate::R270),
        _ => Ok(image.clone()),
    }
}

pub fn ml_module_info() -> Value {
    json!({
        "auto_crop": true,
        "orientation": true,
        "run_user_onnx": true,
        "onnxruntime_available": false,
        "tract_onnx_available": true,
        "cli_worker_isolation": true,
        "bundled_models": [],
        "methods": ["luma-threshold-bbox", "gradient-energy", "user-onnx"],
        "ok": true,
    })
}
