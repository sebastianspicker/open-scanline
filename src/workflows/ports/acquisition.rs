//! Acquisition contracts shared by workflows and concrete scanner adapters.

use crate::domain::acquisition::DeviceOpenPolicy;
use crate::error::Result;
use crate::infrastructure::acquisition::contract::DeviceSession;

/// Scanner-session boundary required by capture workflows.
///
/// Backend discovery and device maintenance are inbound concerns. Capture only
/// needs to resolve a requested device and open a session for it.
pub trait AcquisitionPort: Send + Sync {
    fn resolve_device_id(&self, device: Option<&str>) -> String;

    fn open_device_with_policy(
        &self,
        device_id: &str,
        policy: DeviceOpenPolicy,
    ) -> Result<Box<dyn DeviceSession>>;
}
