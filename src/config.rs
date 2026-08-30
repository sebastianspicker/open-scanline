//! Compatibility facade for the historical OSL-CONFIG path.

pub use crate::infrastructure::config::json::{
    config_from_value, default_config_path, format_curve_points, is_banned_key, load_config,
    parse_curve_points, save_config, strip_banned, validate_hue, validate_output_name,
    validate_saturation, MAX_OUTPUT_NAME_BYTES,
};

pub use crate::workflows::settings::AppConfig;
