//! Windows WIA backend — list/open/scan empty-safe; real COM transfer when device present.
//!
//! The facade preserves public paths while private modules isolate probing,
//! PowerShell command construction, and the stateful COM session adapter.

mod command;
mod probing;
mod session;

pub use command::wia_transfer_command;
pub use probing::{
    available, available_with_cancellation, backend_info, backend_info_with_cancellation,
    list_devices, list_devices_with_cancellation, list_wia_devices_safe,
    list_wia_devices_safe_with_cancellation, parse_wia_devices, SystemCommandRunner,
};
pub use session::{open, open_with_cancellation, WiaDeviceSession};

pub use crate::infrastructure::runtime::{
    ArtifactQuota, CommandOutput, CommandRunner, CommandSpec, ImageDecoder, NativeImageDecoder,
};
