//! Compatibility facade for legacy acquisition imports.
//!
//! New code should depend on `domain::acquisition`, `workflows::ports::acquisition`,
//! `workflows::operation`, or `infrastructure::acquisition` as appropriate.

pub use crate::domain::acquisition::{apply_flat_dark_cal, synthetic_cal_tables, DeviceOpenPolicy};
pub use crate::infrastructure::acquisition::{
    calibrate_device, exposure_from_preview, exposure_gains_from_buffer, find_scanners,
    focus_device, list_all_devices, list_all_devices_with_cancellation, list_backends,
    list_backends_with_cancellation, open_device, open_device_with_policy, resolve_device_id,
    AnySession, FileBackend, FileDeviceSession, MockDevice, MockDeviceSession,
};
pub use crate::workflows::operation::CancellationToken;
pub use crate::workflows::ports::acquisition::{
    BackendInfo, DeviceInfo, DeviceSession, ScanPagesEnd, ScanPagesResult, MAX_SCAN_PAGES,
};
