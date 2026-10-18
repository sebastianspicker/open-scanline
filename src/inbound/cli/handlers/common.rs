use std::path::Path;

pub(super) fn print_wrote(path: &Path) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    println!("wrote {} ({size} bytes)", path.display());
}

/// Point at `--verbose` after a failure unless the diagnostic log is already on.
pub(super) fn print_verbose_hint() {
    if !crate::debug_log::enabled() {
        eprintln!("hint: run again with --verbose for backend diagnostics");
    }
}
