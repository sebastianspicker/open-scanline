//! OS platform helpers (OSL-PLATFORM): paths and environment summary.

use serde_json::{json, Value};
use std::path::PathBuf;

/// Application directory / product name.
pub fn app_name() -> &'static str {
    "open-scanline"
}

/// User-local config directory (`%APPDATA%/open-scanline` or XDG config).
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        dirs::config_dir()
            .unwrap_or_else(|| {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join("AppData")
                            .join("Roaming")
                    })
            })
            .join(app_name())
    }
    #[cfg(not(windows))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| {
                std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".config")
                    })
            })
            .join(app_name())
    }
}

/// User-local data directory for caches, models, resources.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join("AppData")
                            .join("Local")
                    })
            })
            .join(app_name())
    }
    #[cfg(not(windows))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| {
                std::env::var_os("XDG_DATA_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".local")
                            .join("share")
                    })
            })
            .join(app_name())
    }
}

/// Cache directory under platform conventions.
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        data_dir().join("cache")
    }
    #[cfg(not(windows))]
    {
        dirs::cache_dir()
            .unwrap_or_else(|| {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        dirs::home_dir()
                            .unwrap_or_else(|| PathBuf::from("."))
                            .join(".cache")
                    })
            })
            .join(app_name())
    }
}

/// Compact platform summary for diagnostics / CLI info.
pub fn platform_summary() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "app": app_name(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_open_scanline() {
        assert_eq!(app_name(), "open-scanline");
    }

    #[test]
    fn dirs_end_with_app_name() {
        assert!(config_dir().ends_with("open-scanline"));
        assert!(data_dir().ends_with("open-scanline"));
        assert!(cache_dir().to_string_lossy().contains("open-scanline"));
    }

    #[test]
    fn summary_has_required_keys() {
        let v = platform_summary();
        assert_eq!(v["app"], "open-scanline");
        assert!(v.get("os").is_some());
        assert!(v.get("arch").is_some());
    }
}
