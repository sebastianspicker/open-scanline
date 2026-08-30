//! Compatibility facade for native scanner-profile support.

pub use crate::infrastructure::media::{
    apply_scanner_profile, build_icc_bytes, it8_reference_patches, load_scanner_profile,
    make_it8_target_image, profile_scanner_it8, save_profile_json, validate_scanner_profile,
};
