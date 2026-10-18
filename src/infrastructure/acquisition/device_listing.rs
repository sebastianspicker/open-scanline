//! Environment-backed simulated discovery and pipe-list parsing.
//!
//! Used only by the command-backed and network acquisition adapters (SANE,
//! WIA, eSCL); it lives here rather than in `runtime` so that module stays
//! free of acquisition vocabulary.

use super::DeviceInfo;

pub(crate) fn simulate_backends() -> bool {
    matches!(
        std::env::var("OPEN_SCANLINE_SIMULATE_BACKENDS")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Whether `id` must open the simulated session `sim_id`. Simulation is opt-in
/// so a real device that happens to be named `sim` stays openable.
pub(crate) fn opens_simulated_session(id: &str, sim_id: &str, simulation_enabled: bool) -> bool {
    simulation_enabled && id == sim_id
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

#[cfg(test)]
mod tests {
    use super::opens_simulated_session;

    #[test]
    fn simulated_session_requires_the_opt_in() {
        assert!(opens_simulated_session("sane:sim", "sane:sim", true));
        assert!(!opens_simulated_session("sane:sim", "sane:sim", false));
        assert!(!opens_simulated_session("sane:other", "sane:sim", true));
        assert!(!opens_simulated_session("wia:sim", "wia:sim", false));
    }
}
