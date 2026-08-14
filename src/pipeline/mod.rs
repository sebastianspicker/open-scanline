//! Image pipeline ops (OSL-PIPELINE).

mod apply;
mod color;
mod filters;
mod geometry;

pub use apply::apply_pipeline;
pub use color::{
    adjust_brightness_contrast, adjust_hue, adjust_levels, adjust_saturation, apply_curves,
    auto_levels, desaturate, histogram, invert, white_balance,
};
pub use filters::{
    box_blur, colorize, denoise, descreen, flatten, grain_reduction, hole_punch_removal,
    infrared_clean, median_filter, restore_colors, restore_fading, sharpen,
};
pub use geometry::{
    auto_deskew, crop, deskew, estimate_skew_degrees, flip_horizontal, flip_vertical, rotate,
};
