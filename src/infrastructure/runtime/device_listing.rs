//! Environment-backed simulated discovery and pipe-list parsing.

use crate::workflows::ports::acquisition::DeviceInfo;

pub(crate) fn simulate_backends() -> bool {
    matches!(
        std::env::var("OPEN_SCANLINE_SIMULATE_BACKENDS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

pub(crate) fn parse_pipe_devices(
    text: &str,
    id_prefix: &str,
    display_prefix: &str,
    backend: &str,
) -> Vec<DeviceInfo> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (id, name) = line
                .split_once('|')
                .map(|(id, name)| (id.trim(), name.trim()))
                .unwrap_or((line, line));
            (!id.is_empty()).then(|| {
                DeviceInfo::new(
                    format!("{id_prefix}:{id}"),
                    format!("{display_prefix}: {name}"),
                    backend,
                )
            })
        })
        .collect()
}
