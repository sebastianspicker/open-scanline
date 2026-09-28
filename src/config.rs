//! Compatibility facade for the historical OSL-CONFIG path.

pub use crate::domain::settings::{
    format_curve_points, is_banned_key, parse_curve_points, strip_banned, validate_hue,
    validate_output_name, validate_saturation, MAX_OUTPUT_NAME_BYTES,
};
pub use crate::infrastructure::config::json::{
    config_from_value, default_config_path, load_config, save_config,
};

pub use crate::infrastructure::config::AppConfig;
