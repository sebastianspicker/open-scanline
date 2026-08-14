use super::{DeviceInfo, DeviceSession};
use crate::core::{ImageBuffer, Result, ScanError, ScanRequest};
use crate::imaging::load_image;
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
    closed: Mutex<bool>,
    cancelled: Mutex<bool>,
    source: Mutex<Option<ImageBuffer>>,
}

impl FileDeviceSession {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            closed: Mutex::new(false),
            cancelled: Mutex::new(false),
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
    pub fn resize_nearest(image: &ImageBuffer, width: u32, height: u32) -> Result<ImageBuffer> {
        if width == image.width && height == image.height {
            return Ok(image.clone());
        }
        let bytes_per_pixel = image.bpp();
        let output_len = crate::core::checked_image_len(width, height, bytes_per_pixel)?;
        let mut output = vec![0_u8; output_len];
        let source_width = image.width as usize;
        let source_height = image.height as usize;
        for y in 0..height as usize {
            let source_y = y * source_height / height as usize;
            for x in 0..width as usize {
                let source_x = x * source_width / width as usize;
                let source_index = (source_y * source_width + source_x) * bytes_per_pixel;
                let output_index = (y * width as usize + x) * bytes_per_pixel;
                output[output_index..output_index + bytes_per_pixel]
                    .copy_from_slice(&image.data[source_index..source_index + bytes_per_pixel]);
            }
        }
        ImageBuffer::new(width, height, image.pixel_format, output)
    }
}

impl DeviceSession for FileDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        super::reject_single_page_duplex(request)?;
        if *self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return Err(ScanError::Other("session closed".into()));
        }
        if *self
            .cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let mut image = self.load()?;
        if request.width > 0 && request.height > 0 {
            image = Self::resize_nearest(&image, request.width, request.height)?;
        }
        if let Some(region) = request.region {
            image = crate::pipeline::crop(&image, region)?;
        }
        Ok(image)
    }

    fn cancel(&self) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            *cancelled = true;
        }
    }

    fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            *closed = true;
        }
        if let Ok(mut source) = self.source.lock() {
            *source = None;
        }
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
