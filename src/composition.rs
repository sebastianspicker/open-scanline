//! Native runtime composition for inbound workflows.

use crate::infrastructure::acquisition::NativeAcquisition;
use crate::infrastructure::media::NativeMedia;

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
