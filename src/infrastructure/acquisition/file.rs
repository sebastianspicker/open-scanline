use crate::domain::acquisition::reject_single_page_duplex;
use crate::domain::acquisition::ScanRequest;
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::{DeviceInfo, DeviceSession};
use crate::infrastructure::media::load_image;
use crate::infrastructure::runtime::CommandSession;
use std::path::PathBuf;
use std::sync::Mutex;

/// File-image backend: device id `file:PATH` or `file://PATH`.
pub struct FileBackend;

impl FileBackend {
    pub fn list_devices() -> Vec<DeviceInfo> {
        let path_text = match std::env::var("OPEN_SCANLINE_FILE_DEVICE") {
            Ok(value) => value.trim().to_string(),
            Err(_) => return Vec::new(),
        };
        if path_text.is_empty() {
            return Vec::new();
        }
        let path = PathBuf::from(&path_text);
        if !path.is_file() {
            return Vec::new();
        }
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_text.clone());
        let id = format!("file:{}", path.canonicalize().unwrap_or(path).display());
        vec![DeviceInfo::new(id, format!("File source ({name})"), "file")]
    }
}

pub struct FileDeviceSession {
    path: PathBuf,
    session: CommandSession,
    source: Mutex<Option<ImageBuffer>>,
}

impl FileDeviceSession {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            session: CommandSession::default(),
            source: Mutex::new(None),
        }
    }

    fn load(&self) -> Result<ImageBuffer> {
        let mut source = self
            .source
            .lock()
            .map_err(|_| ScanError::Other("file session lock poisoned".into()))?;
        if source.is_none() {
            if !self.path.is_file() {
                return Err(ScanError::DeviceNotFound(format!(
                    "file device source missing: {}",
                    self.path.display()
                )));
            }
            *source = Some(load_image(&self.path)?);
        }
        Ok(source.as_ref().expect("source was initialized").clone())
    }

    /// Nearest-neighbor resize to request width/height.
    ///
    /// Kept as a delegating associated function because it is reachable
    /// through the `device` facade; the pure algorithm now lives in
    /// [`crate::domain::processing::resize_nearest`].
    pub fn resize_nearest(image: &ImageBuffer, width: u32, height: u32) -> Result<ImageBuffer> {
        crate::domain::processing::resize_nearest(image, width, height)
    }
}

impl DeviceSession for FileDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        super::batch::validate_session_state(
            self.session.is_closed(),
            self.session.is_cancelled(),
        )?;
        self.apply_request(self.load()?, request)
    }

    fn cancel(&self) {
        self.session.cancel();
    }

    fn close(&self) {
        self.session.close();
        if let Ok(mut source) = self.source.lock() {
            *source = None;
        }
    }
}

impl FileDeviceSession {
    fn apply_request(&self, image: ImageBuffer, request: &ScanRequest) -> Result<ImageBuffer> {
        let image = resize_for_request(image, request)?;
        match request.region {
            Some(region) => crate::domain::processing::crop(&image, region),
            None => Ok(image),
        }
    }
}

fn resize_for_request(image: ImageBuffer, request: &ScanRequest) -> Result<ImageBuffer> {
    let (width, height) = fit_extent(image.width, image.height, request.width, request.height);
    if (width, height) == (image.width, image.height) {
        return Ok(image);
    }
    crate::domain::processing::resize_nearest(&image, width, height)
}

/// Size of a `source` image scaled to fit the requested extent while keeping
/// its aspect ratio. Both axes set form a bounding box; one axis set scales to
/// that axis; neither set keeps the source size. Zero means unconstrained.
fn fit_extent(source_width: u32, source_height: u32, width: u32, height: u32) -> (u32, u32) {
    if source_width == 0 || source_height == 0 {
        return (source_width, source_height);
    }
    let scaled = |value: u64, numerator: u64, denominator: u64| {
        ((value * numerator + denominator / 2) / denominator).clamp(1, u64::from(u32::MAX)) as u32
    };
    let (sw, sh) = (u64::from(source_width), u64::from(source_height));
    let (rw, rh) = (u64::from(width), u64::from(height));
    match (width, height) {
        (0, 0) => (source_width, source_height),
        (_, 0) => (width, scaled(sh, rw, sw)),
        (0, _) => (scaled(sw, rh, sh), height),
        _ if rw * sh <= rh * sw => (width, scaled(sh, rw, sw)),
        _ => (scaled(sw, rh, sh), height),
    }
}

pub(super) fn parse_file_device_id(device_id: &str) -> PathBuf {
    let raw = if let Some(rest) = device_id.strip_prefix("file://") {
        rest
    } else if let Some(rest) = device_id.strip_prefix("file:") {
        rest
    } else {
        device_id
    };
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::fit_extent;

    #[test]
    fn extent_is_a_bounding_box_that_keeps_the_aspect_ratio() {
        // 4:3 source into a portrait A4-like box is width-limited.
        assert_eq!(fit_extent(400, 300, 200, 400), (200, 150));
        // Height-limited.
        assert_eq!(fit_extent(400, 300, 800, 150), (200, 150));
        // Upscaling fits the box too.
        assert_eq!(fit_extent(100, 50, 400, 400), (400, 200));
        // Exact aspect keeps the box.
        assert_eq!(fit_extent(400, 300, 320, 240), (320, 240));
    }

    #[test]
    fn single_axis_scales_to_that_axis_and_none_keeps_the_source() {
        assert_eq!(fit_extent(400, 300, 200, 0), (200, 150));
        assert_eq!(fit_extent(400, 300, 0, 150), (200, 150));
        assert_eq!(fit_extent(400, 300, 0, 0), (400, 300));
        assert_eq!(fit_extent(1000, 1, 10, 0), (10, 1));
    }
}
