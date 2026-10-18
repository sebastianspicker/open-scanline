//! Opt-in diagnostic log written to stderr.
//!
//! Enabled by `--verbose` or `OPEN_SCANLINE_DEBUG`. Never log secrets such as
//! PDF passwords.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub(crate) fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Enable the log when `OPEN_SCANLINE_DEBUG` is set to a value other than `""`/`0`.
pub(crate) fn init_from_env() {
    if env_value_enables(std::env::var("OPEN_SCANLINE_DEBUG").ok().as_deref()) {
        enable();
    }
}

fn env_value_enables(value: Option<&str>) -> bool {
    value.is_some_and(|value| !matches!(value.trim(), "" | "0"))
}

/// Print `open-scanline debug [<area>] <message>` to stderr when enabled.
macro_rules! debug_log {
    ($area:expr, $($arg:tt)*) => {
        if $crate::debug_log::enabled() {
            eprintln!("open-scanline debug [{}] {}", $area, format_args!($($arg)*));
        }
    };
}
pub(crate) use debug_log;

#[cfg(test)]
mod tests {
    use super::env_value_enables;

    #[test]
    fn env_values_enable_unless_empty_or_zero() {
        assert!(!env_value_enables(None));
        assert!(!env_value_enables(Some("")));
        assert!(!env_value_enables(Some("0")));
        assert!(env_value_enables(Some("1")));
        assert!(env_value_enables(Some("true")));
    }
}
