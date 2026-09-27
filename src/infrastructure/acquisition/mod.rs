//! Concrete acquisition registry and scanner adapters.

mod batch;
mod command_backend;
pub mod contract;
mod device_listing;
#[cfg(feature = "gui")]
mod discovery;
#[cfg(feature = "gui")]
pub use discovery::maintenance_capabilities_with_cancellation;
pub mod escl;

pub mod file;
pub mod mock;
mod registry;
pub mod sane;
pub mod wia;

pub use contract::{
    BackendInfo, DeviceInfo, DeviceMaintenanceCapabilities, DeviceSession, FocusCapability,
    MaintenanceAvailability, ScanPagesEnd, ScanPagesResult,
};
pub(crate) use device_listing::{parse_pipe_devices, simulate_backends};
pub use registry::*;
