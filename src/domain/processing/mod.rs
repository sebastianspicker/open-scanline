//! Pure processing plans and image transformation algorithms.

mod apply;
mod auto;
mod color;
mod film;
mod filters;
mod geometry;
mod plan;

pub use apply::apply_pipeline;
pub use auto::{
    apply_auto_crop, apply_auto_crop_with_params, apply_orientation,
    apply_orientation_with_degrees, auto_crop_bounds, detect_orientation, AutoCropResult,
    OrientationResult,
};
pub use color::{
    adjust_brightness_contrast, adjust_hue, adjust_levels, adjust_saturation, apply_curves,
    auto_levels, desaturate, histogram, invert, white_balance,
};
pub use film::{convert_film, get_film_profile, list_film_profiles, FilmProfile};
pub use filters::{
    box_blur, colorize, denoise, descreen, flatten, grain_reduction, hole_punch_removal,
    infrared_clean, median_filter, restore_colors, restore_fading, sharpen,
};
pub use geometry::{
    auto_deskew, crop, deskew, estimate_skew_degrees, flip_horizontal, flip_vertical, rotate,
};
pub use plan::PipelinePrefs;
