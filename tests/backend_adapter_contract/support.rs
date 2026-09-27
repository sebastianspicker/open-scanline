pub(crate) use open_scanline::core::{
    ImageBuffer, PixelFormat, Result, ScanError, ScanMode, ScanRequest,
};
pub(crate) use open_scanline::device::{DeviceSession, ScanPagesEnd};
pub(crate) use open_scanline::{sane, wia};
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier, Mutex, OnceLock,
};
pub(crate) use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) static WIA_OUTPUT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn fixture_image() -> ImageBuffer {
    ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![10, 20, 30, 40, 50, 60]).unwrap()
}

pub(crate) fn output_path_from_wia(script: &str) -> PathBuf {
    let prefix = "$out = '";
    let start = script.find(prefix).unwrap() + prefix.len();
    let end = script[start..].find('\'').unwrap() + start;
    PathBuf::from(&script[start..end])
}
