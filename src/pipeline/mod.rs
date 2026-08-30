//! Compatibility facade for the historical OSL-PIPELINE path.

pub use crate::domain::processing::{
    adjust_brightness_contrast, adjust_hue, adjust_levels, adjust_saturation, apply_auto_crop,
    apply_auto_crop_with_params, apply_curves, apply_orientation, apply_orientation_with_degrees,
    apply_pipeline, auto_crop_bounds, auto_deskew, auto_levels, box_blur, colorize, crop, denoise,
    desaturate, descreen, deskew, detect_orientation, estimate_skew_degrees, flatten,
    flip_horizontal, flip_vertical, grain_reduction, histogram, hole_punch_removal, infrared_clean,
    invert, median_filter, restore_colors, restore_fading, rotate, sharpen, white_balance,
    AutoCropResult, OrientationResult,
};
