//! SANE backend — empty-safe with scanimage list/open/scan when present.
//!
//! The facade keeps the established public module path while private modules
//! isolate probing, option parsing, command construction, and session state.

mod capabilities;
mod command;
mod pages;
mod probing;
mod session;

pub use capabilities::{
    parse_scanimage_help, parse_scanimage_maintenance_options, scanimage_maintenance_command,
    scanimage_maintenance_inspection_command, SaneCapabilities, SaneMaintenanceAction,
    SaneMaintenanceOptions,
};
pub use command::scanimage_command;
pub(crate) use pages::{emit_simulated_pages, emit_single_pages};
pub use probing::{
    available, available_with_cancellation, backend_info, backend_info_with_cancellation,
    list_devices, list_devices_with_cancellation, list_sane_devices_safe,
    list_sane_devices_safe_with_cancellation, parse_sane_devices, SystemCommandRunner,
};
pub use session::{open, open_with_cancellation, SaneDeviceSession};

pub use crate::infrastructure::media::NativeImageDecoder;
pub use crate::infrastructure::runtime::{
    ArtifactQuota, CommandOutput, CommandRunner, CommandSpec, ImageDecoder,
};
