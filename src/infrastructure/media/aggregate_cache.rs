//! Export-scoped cache for decoded page pixels shared by aggregate writers.

use super::load_image;
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::TemporaryOutput;
use crate::operation::CancellationToken;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CACHE_MAGIC: &[u8; 8] = b"OSLPIX01";
const CACHE_HEADER_BYTES: usize = 8 + 4 + 4 + 1 + 8;
const MAX_CACHE_BYTES: usize = 512 * 1024 * 1024;

#[cfg(test)]
mod benchmark;

#[derive(Debug)]
struct CacheEntry {
    source_hash: [u8; 32],
    path: PathBuf,
    bytes: usize,
}

/// Pages are decoded lazily and cached only as tightly packed pixels. Cache
/// failures fall back to the ordinary decoder, while source read/decode errors
/// remain visible to the caller.
#[derive(Debug)]
pub(crate) struct SharedPageLoader {
    storage: Option<TemporaryOutput>,
    entries: HashMap<PathBuf, CacheEntry>,
    used_bytes: usize,
    limit: usize,
    sequence: usize,
    #[cfg(test)]
    decode_count: usize,
}

impl SharedPageLoader {
    pub(crate) fn new() -> Self {
        Self::with_limit(MAX_CACHE_BYTES)
    }

    fn with_limit(limit: usize) -> Self {
        Self {
            storage: TemporaryOutput::new("aggregate-pixels", "cache").ok(),
            entries: HashMap::new(),
            used_bytes: 0,
            limit,
            sequence: 0,
            #[cfg(test)]
            decode_count: 0,
        }
    }

    pub(crate) fn load(
        &mut self,
        source: &Path,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ImageBuffer> {
        self.load_with(source, cancellation, |path| load_image(path))
    }

    fn load_with<F>(
        &mut self,
        source: &Path,
        cancellation: Option<&CancellationToken>,
        decode: F,
    ) -> Result<ImageBuffer>
    where
        F: FnOnce(&Path) -> Result<ImageBuffer>,
    {
        check_cancellation(cancellation)?;
        if self.storage.is_none() {
            #[cfg(test)]
            {
                self.decode_count += 1;
            }
            return decode(source);
        }
        let source_hash = hash_source(source, cancellation)?;
        if let Some(image) = self.read_matching(source, source_hash, cancellation)? {
            check_cancellation(cancellation)?;
            return Ok(image);
        }
        self.decode_and_store(source, source_hash, cancellation, decode)
    }

    fn decode_and_store<F>(
        &mut self,
        source: &Path,
        source_hash: [u8; 32],
        cancellation: Option<&CancellationToken>,
        decode: F,
    ) -> Result<ImageBuffer>
    where
        F: FnOnce(&Path) -> Result<ImageBuffer>,
    {
        self.invalidate(source);
        #[cfg(test)]
        {
            self.decode_count += 1;
        }
        let image = decode(source)?;
        check_cancellation(cancellation)?;
        let confirmed_hash = hash_source(source, cancellation)?;
        if confirmed_hash == source_hash {
            self.try_store(source, source_hash, &image, cancellation)?;
        }
        check_cancellation(cancellation)?;
        Ok(image)
    }

    fn read_matching(
        &self,
        source: &Path,
        source_hash: [u8; 32],
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<ImageBuffer>> {
        let Some(entry) = self.entries.get(source) else {
            return Ok(None);
        };
        if entry.source_hash != source_hash {
            return Ok(None);
        }
        match read_cached_image(&entry.path, entry.bytes, cancellation) {
            Ok(image) => {
                let confirmed = hash_source(source, cancellation)?;
                Ok((confirmed == source_hash).then_some(image))
            }
            Err(error @ ScanError::Cancelled(_)) => Err(error),
            Err(_) => Ok(None),
        }
    }

    fn invalidate(&mut self, source: &Path) {
        if let Some(entry) = self.entries.remove(source) {
            if std::fs::remove_file(entry.path).is_ok() {
                self.used_bytes = self.used_bytes.saturating_sub(entry.bytes);
            }
        }
    }

    fn try_store(
        &mut self,
        source: &Path,
        source_hash: [u8; 32],
        image: &ImageBuffer,
        cancellation: Option<&CancellationToken>,
    ) -> Result<()> {
        let Some(storage) = self.storage.as_ref() else {
            return Ok(());
        };
        let Some(bytes) = CACHE_HEADER_BYTES.checked_add(image.data.len()) else {
            return Ok(());
        };
        if self.used_bytes.saturating_add(bytes) > self.limit {
            return Ok(());
        }
        let path = storage
            .directory()
            .join(format!("page-{:06}.pixels", self.sequence));
        self.sequence += 1;
        if let Err(error) = write_cached_image(&path, image, cancellation) {
            self.remove_or_account_partial(&path);
            return match error {
                error @ ScanError::Cancelled(_) => Err(error),
                _ => Ok(()),
            };
        }
        if !matches!(std::fs::metadata(&path), Ok(meta) if meta.len() == bytes as u64) {
            self.remove_or_account_partial(&path);
            return Ok(());
        }
        self.used_bytes += bytes;
        self.entries.insert(
            source.to_path_buf(),
            CacheEntry {
                source_hash,
                path,
                bytes,
            },
        );
        Ok(())
    }

    fn remove_or_account_partial(&mut self, path: &Path) {
        if std::fs::remove_file(path).is_ok() {
            return;
        }
        if let Ok(metadata) = std::fs::metadata(path) {
            let bytes = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            self.used_bytes = self.used_bytes.saturating_add(bytes);
        }
    }

    #[cfg(test)]
    fn directory(&self) -> Option<&Path> {
        self.storage.as_ref().map(TemporaryOutput::directory)
    }

    #[cfg(test)]
    pub(crate) fn decode_count(&self) -> usize {
        self.decode_count
    }
}

fn hash_source(path: &Path, cancellation: Option<&CancellationToken>) -> Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_cancellation(cancellation)?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    check_cancellation(cancellation)?;
    Ok(hasher.finalize().into())
}

fn write_cached_image(
    path: &Path,
    image: &ImageBuffer,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    let mut file = File::options().write(true).create_new(true).open(path)?;
    write_cache_header(&mut file, image)?;
    write_all_cancelled(&mut file, &image.data, cancellation)?;
    file.flush()?;
    Ok(())
}

fn write_all_cancelled(
    file: &mut File,
    bytes: &[u8],
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    for chunk in bytes.chunks(64 * 1024) {
        check_cancellation(cancellation)?;
        file.write_all(chunk)?;
    }
    check_cancellation(cancellation)
}

fn write_cache_header(file: &mut File, image: &ImageBuffer) -> Result<()> {
    let mut header = Vec::with_capacity(CACHE_HEADER_BYTES);
    header.extend_from_slice(CACHE_MAGIC);
    header.extend_from_slice(&image.width.to_le_bytes());
    header.extend_from_slice(&image.height.to_le_bytes());
    header.push(format_tag(image.pixel_format));
    header.extend_from_slice(&(image.data.len() as u64).to_le_bytes());
    file.write_all(&header)?;
    Ok(())
}

fn read_cached_image(
    path: &Path,
    expected_bytes: usize,
    cancellation: Option<&CancellationToken>,
) -> Result<ImageBuffer> {
    validate_cache_file_size(path, expected_bytes)?;
    let mut file = File::open(path)?;
    let header = read_cache_header(&mut file)?;
    if &header[..8] != CACHE_MAGIC {
        return Err(ScanError::Invalid(
            "decoded-page cache header is invalid".into(),
        ));
    }
    let (width, height, format, expected_pixel_bytes) =
        parse_cache_header(&header, expected_bytes)?;
    let mut data = vec![0_u8; expected_pixel_bytes];
    read_exact_cancelled(&mut file, &mut data, cancellation)?;
    ImageBuffer::new(width, height, format, data)
}

fn validate_cache_file_size(path: &Path, expected_bytes: usize) -> Result<()> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() == expected_bytes as u64 && expected_bytes <= MAX_CACHE_BYTES {
        Ok(())
    } else {
        Err(ScanError::Invalid(
            "decoded-page cache size is invalid".into(),
        ))
    }
}

fn read_cache_header(file: &mut File) -> Result<[u8; CACHE_HEADER_BYTES]> {
    let mut header = [0_u8; CACHE_HEADER_BYTES];
    file.read_exact(&mut header)?;
    Ok(header)
}

fn parse_cache_header(
    header: &[u8; CACHE_HEADER_BYTES],
    expected_bytes: usize,
) -> Result<(u32, u32, PixelFormat, usize)> {
    let width = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let height = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let format = parse_format(header[16])?;
    let length = u64::from_le_bytes(header[17..25].try_into().unwrap());
    let pixel_bytes = crate::domain::image::checked_image_len(width, height, format.bpp())?;
    if length != pixel_bytes as u64 || expected_bytes != CACHE_HEADER_BYTES + pixel_bytes {
        return Err(ScanError::Invalid(
            "decoded-page cache length is invalid".into(),
        ));
    }
    Ok((width, height, format, pixel_bytes))
}

fn read_exact_cancelled(
    file: &mut File,
    bytes: &mut [u8],
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    for chunk in bytes.chunks_mut(64 * 1024) {
        check_cancellation(cancellation)?;
        file.read_exact(chunk)?;
    }
    check_cancellation(cancellation)
}

fn format_tag(format: PixelFormat) -> u8 {
    match format {
        PixelFormat::Gray8 => 1,
        PixelFormat::Rgb8 => 2,
        PixelFormat::Rgba8 => 3,
    }
}

fn parse_format(tag: u8) -> Result<PixelFormat> {
    match tag {
        1 => Ok(PixelFormat::Gray8),
        2 => Ok(PixelFormat::Rgb8),
        3 => Ok(PixelFormat::Rgba8),
        _ => Err(ScanError::Invalid(
            "decoded-page cache pixel format is invalid".into(),
        )),
    }
}

fn check_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled(
            "aggregate page loading cancelled".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch_source(label: &str, bytes: &[u8]) -> PathBuf {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-cache-{label}-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn fixture(value: u8) -> ImageBuffer {
        ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![value; 3]).unwrap()
    }

    #[test]
    fn four_aggregate_passes_decode_each_lossy_source_once() {
        let source = scratch_source("lossy", b"lossy encoded source");
        let decodes = Cell::new(0_u32);
        let mut loader = SharedPageLoader::new();
        for _ in 0..4 {
            let image = loader
                .load_with(&source, None, |_| {
                    decodes.set(decodes.get() + 1);
                    Ok(fixture(91))
                })
                .unwrap();
            assert_eq!(image, fixture(91));
        }
        assert_eq!(decodes.get(), 1);
        let _ = std::fs::remove_file(source);
    }

    #[test]
    fn actual_jpeg_decodes_once_and_cache_replays_identical_lossy_pixels() {
        let placeholder = scratch_source("actual-lossy", b"placeholder");
        let source = placeholder.with_extension("jpg");
        std::fs::remove_file(placeholder).unwrap();
        let expected_source = fixture(137);
        crate::infrastructure::media::save_image(&source, &expected_source, None, Some(72))
            .unwrap();
        let decodes = Cell::new(0_u32);
        let mut loader = SharedPageLoader::new();
        let first = loader
            .load_with(&source, None, |path| {
                decodes.set(decodes.get() + 1);
                load_image(path)
            })
            .unwrap();
        for _ in 0..3 {
            let replay = loader
                .load_with(&source, None, |path| {
                    decodes.set(decodes.get() + 1);
                    load_image(path)
                })
                .unwrap();
            assert_eq!(replay, first);
        }
        assert_eq!(decodes.get(), 1);
        let _ = std::fs::remove_file(source);
    }

    #[test]
    fn changed_source_and_corrupt_cache_use_the_ordinary_decoder() {
        let source = scratch_source("changed", b"first");
        let decodes = Cell::new(0_u32);
        let mut loader = SharedPageLoader::new();
        let decode = |_: &Path| {
            decodes.set(decodes.get() + 1);
            Ok(fixture(decodes.get() as u8))
        };
        assert_eq!(loader.load_with(&source, None, decode).unwrap(), fixture(1));
        std::fs::write(&source, b"second").unwrap();
        assert_eq!(loader.load_with(&source, None, decode).unwrap(), fixture(2));
        let cache_path = loader.entries.get(&source).unwrap().path.clone();
        std::fs::write(cache_path, b"broken").unwrap();
        assert_eq!(loader.load_with(&source, None, decode).unwrap(), fixture(3));
        let _ = std::fs::remove_file(source);
    }

    #[test]
    fn quota_and_cache_write_failure_fall_back_without_hiding_source_errors() {
        let source = scratch_source("fallback", b"source");
        let decodes = Cell::new(0_u32);
        let mut loader = SharedPageLoader::with_limit(1);
        for _ in 0..2 {
            loader
                .load_with(&source, None, |_| {
                    decodes.set(decodes.get() + 1);
                    Ok(fixture(1))
                })
                .unwrap();
        }
        assert_eq!(decodes.get(), 2);

        let second = scratch_source("write-failure", b"source");
        let mut broken = SharedPageLoader::new();
        std::fs::remove_dir_all(broken.directory().unwrap()).unwrap();
        let first = broken.load_with(&second, None, |_| Ok(fixture(4))).unwrap();
        let next = broken.load_with(&second, None, |_| Ok(fixture(5))).unwrap();
        assert_eq!(first, fixture(4));
        assert_eq!(next, fixture(5));
        let _ = std::fs::remove_file(second);

        std::fs::remove_file(&source).unwrap();
        assert!(loader.load_with(&source, None, |_| Ok(fixture(1))).is_err());
    }

    #[test]
    fn cancellation_and_panic_drop_private_cache_directory() {
        let source = scratch_source("cleanup", b"source");
        let token = CancellationToken::new();
        token.cancel();
        let mut loader = SharedPageLoader::new();
        assert!(matches!(
            loader.load_with(&source, Some(&token), |_| Ok(fixture(1))),
            Err(ScanError::Cancelled(_))
        ));
        let directory = loader.directory().unwrap().to_path_buf();
        let _ = std::panic::catch_unwind(move || {
            let _owned = loader;
            panic!("exercise cache RAII");
        });
        assert!(!directory.exists());
        let _ = std::fs::remove_file(source);
    }

    #[test]
    fn cancellation_during_cache_store_removes_partial_file() {
        let source = scratch_source("store-cancel", b"source");
        let source_hash = hash_source(&source, None).unwrap();
        let token = CancellationToken::new();
        token.cancel();
        let mut loader = SharedPageLoader::new();
        let error = loader
            .try_store(&source, source_hash, &fixture(9), Some(&token))
            .unwrap_err();
        assert!(matches!(error, ScanError::Cancelled(_)));
        assert_eq!(loader.used_bytes, 0);
        assert!(loader.entries.is_empty());
        assert_eq!(
            std::fs::read_dir(loader.directory().unwrap())
                .unwrap()
                .count(),
            0
        );
        let _ = std::fs::remove_file(source);
    }

    #[cfg(unix)]
    #[test]
    fn failed_stale_file_removal_does_not_release_quota() {
        use std::os::unix::fs::PermissionsExt;

        let source = scratch_source("stale-quota", b"source");
        let mut loader = SharedPageLoader::new();
        loader.load_with(&source, None, |_| Ok(fixture(3))).unwrap();
        let used = loader.used_bytes;
        let directory = loader.directory().unwrap().to_path_buf();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        loader.invalidate(&source);
        assert_eq!(loader.used_bytes, used);
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_file(source);
    }
}
