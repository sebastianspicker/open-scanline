//! Native runtime composition for inbound workflows.

use crate::domain::acquisition::DeviceOpenPolicy;
use crate::error::Result;
use crate::infrastructure::acquisition::{
    open_device_with_policy, resolve_device_id, DeviceSession,
};
use crate::infrastructure::media::NativeMedia;
use crate::workflows::ports::acquisition::AcquisitionPort;

/// Native scanner registry adapter supplied by [`Runtime`].
///
/// This is the wiring seam between the `AcquisitionPort` workflow port and the
/// concrete infrastructure acquisition adapters; only composition may depend
/// on both layers.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeAcquisition;

impl AcquisitionPort for NativeAcquisition {
    fn resolve_device_id(&self, device: Option<&str>) -> String {
        resolve_device_id(device)
    }

    fn open_device_with_policy(
        &self,
        device_id: &str,
        policy: DeviceOpenPolicy,
    ) -> Result<Box<dyn DeviceSession>> {
        Ok(Box::new(open_device_with_policy(device_id, policy)?))
    }
}

/// Production implementations supplied to workflows at the application edge.
///
/// This is deliberately small: workflow code receives the grouped ports it
/// needs and never selects scanner backends, codecs, or persistence adapters.
#[derive(Debug, Default, Clone, Copy)]
pub struct Runtime {
    acquisition: NativeAcquisition,
    media: NativeMedia,
}

impl Runtime {
    pub const fn acquisition(&self) -> &NativeAcquisition {
        &self.acquisition
    }

    pub const fn media(&self) -> &NativeMedia {
        &self.media
    }
}
