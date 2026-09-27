//! Concrete acquisition registry and scanner adapters.

mod batch;
pub mod contract;
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
pub use registry::*;
