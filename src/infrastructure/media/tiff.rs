use super::codecs::{create_output_temp, validate_output_container};
use super::load_image;
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

mod append;
pub use append::append_page_to_multipage;

const TIFF_IFD_ENTRY_COUNT: u16 = 12;
const TIFF_IFD_SIZE: u32 = 2 + TIFF_IFD_ENTRY_COUNT as u32 * 12 + 4;
const TIFF_HEADER_SIZE: u32 = 8;
const TIFF_WORD_ALIGNMENT: u32 = 2;

#[derive(Debug)]
struct TiffPageLayout {
    ifd_at: u32,
    bps_at: u32,
    xres_at: u32,
    yres_at: u32,
    strip_at: u32,
    strip_len: u32,
    next_ifd_at: u32,
    next_ifd_field_at: u32,
}

impl TiffPageLayout {
    fn new(ifd_at: u32, strip_len: u32, has_next_page: bool) -> Result<Self> {
        let ifd_at = align_tiff_offset(ifd_at)?;
        let (bps_at, xres_at, yres_at, strip_at, strip_end) = page_data_offsets(ifd_at, strip_len)?;
        let next_ifd_at = if has_next_page {
            align_tiff_offset(strip_end)?
        } else {
            0
        };
        let next_ifd_field_at = checked_tiff_offset(ifd_at, TIFF_IFD_SIZE - 4)?;
        Ok(Self {
            ifd_at,
            bps_at,
            xres_at,
            yres_at,
            strip_at,
            strip_len,
            next_ifd_at,
            next_ifd_field_at,
        })
    }
}

fn page_data_offsets(ifd_at: u32, strip_len: u32) -> Result<(u32, u32, u32, u32, u32)> {
    let bps_at = checked_tiff_offset(ifd_at, TIFF_IFD_SIZE)?;
    let xres_at = checked_tiff_offset(bps_at, 6)?;
    let yres_at = checked_tiff_offset(xres_at, 8)?;
    let strip_at = checked_tiff_offset(yres_at, 8)?;
    Ok((
        bps_at,
        xres_at,
        yres_at,
        strip_at,
        checked_tiff_offset(strip_at, strip_len)?,
    ))
}

fn checked_tiff_offset(offset: u32, length: u32) -> Result<u32> {
    offset
        .checked_add(length)
        .ok_or_else(|| ScanError::Image("TIFF layout exceeds classic TIFF offset limit".into()))
}

fn align_tiff_offset(offset: u32) -> Result<u32> {
    checked_tiff_offset(offset, offset % TIFF_WORD_ALIGNMENT)
}

fn checked_tiff_frame_aggregate(
    current_rgb_bytes: usize,
    width: u32,
    height: u32,
) -> Result<usize> {
    let frame_rgb_bytes =
        crate::domain::image::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
    let aggregate_rgb_bytes = current_rgb_bytes
        .checked_add(frame_rgb_bytes)
        .ok_or_else(|| {
            ScanError::Image("TIFF decoded frame aggregate exceeds the image safety limit".into())
        })?;
    if aggregate_rgb_bytes > crate::domain::image::MAX_IMAGE_BYTES {
        return Err(ScanError::Image(
            "TIFF decoded frame aggregate exceeds the image safety limit".into(),
        ));
    }
    Ok(aggregate_rgb_bytes)
}

/// Combine on-disk page images into one multipage TIFF (real multi-IFD).
pub fn save_multipage_tiff(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    dpi: Option<u32>,
) -> Result<PathBuf> {
    save_multipage_tiff_with_cancellation(pages, out, dpi, None)
}

/// Combine page images while allowing cancellation before each page and publication.
pub fn save_multipage_tiff_with_cancellation(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    dpi: Option<u32>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_multipage_tiff_from_paths_with_transform_and_cancellation(
        pages,
        out,
        dpi,
        |image| Ok(image.clone()),
        cancellation,
    )
}

/// Combine page files after applying an export-time transform to every page.
pub fn save_multipage_tiff_from_paths_with_transform<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    dpi: Option<u32>,
    transform: F,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    save_multipage_tiff_from_paths_with_transform_and_cancellation(pages, out, dpi, transform, None)
}

/// Combine transformed page files while allowing cancellation before every
/// page, transform, TIFF append, and atomic publication.
pub fn save_multipage_tiff_from_paths_with_transform_and_cancellation<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    dpi: Option<u32>,
    mut transform: F,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    save_multipage_tiff_with_loader_and_transform(
        pages,
        out.as_ref(),
        dpi,
        |path| load_image(path),
        |image| transform(&image),
        cancellation,
    )
}

pub(crate) fn save_multipage_tiff_with_loader_and_transform<L, F>(
    pages: &[PathBuf],
    out: &Path,
    dpi: Option<u32>,
    mut load: L,
    mut transform: F,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    validate_tiff_path_request(pages, cancellation)?;
    create_tiff_parent(out)?;
    let temp = create_output_temp(out)?;
    write_and_publish_tiff(temp, pages, dpi, &mut load, &mut transform, cancellation)?;
    Ok(out.to_path_buf())
}

fn validate_tiff_path_request(
    pages: &[PathBuf],
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    if pages.is_empty() {
        return Err(ScanError::Invalid(
            "save_multipage_tiff requires at least one page".into(),
        ));
    }
    check_tiff_cancellation(cancellation)
}
fn write_and_publish_tiff<L, F>(
    temp: super::codecs::OutputTemp,
    pages: &[PathBuf],
    dpi: Option<u32>,
    load: &mut L,
    transform: &mut F,
    cancellation: Option<&CancellationToken>,
) -> Result<()>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    let mut writer = TiffStreamWriter::new(temp.path())?;
    write_transformed_tiff_pages(&mut writer, pages, dpi, load, transform, cancellation)?;
    writer.finish()?;
    drop(writer);
    validate_output_container(temp.path(), "tiff")?;
    check_tiff_cancellation(cancellation)?;
    temp.publish()
}

fn create_tiff_parent(out: &Path) -> Result<()> {
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn write_transformed_tiff_pages<L, F>(
    writer: &mut TiffStreamWriter,
    pages: &[PathBuf],
    dpi: Option<u32>,
    load: &mut L,
    transform: &mut F,
    cancellation: Option<&CancellationToken>,
) -> Result<()>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    for (page_index, path) in pages.iter().enumerate() {
        write_transformed_tiff_page(
            writer,
            path,
            page_index + 1 < pages.len(),
            dpi,
            load,
            transform,
            cancellation,
        )?;
    }
    Ok(())
}

fn write_transformed_tiff_page<L, F>(
    writer: &mut TiffStreamWriter,
    path: &Path,
    has_next: bool,
    dpi: Option<u32>,
    load: &mut L,
    transform: &mut F,
    cancellation: Option<&CancellationToken>,
) -> Result<()>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    check_tiff_cancellation(cancellation)?;
    let image = load(path)?;
    check_tiff_cancellation(cancellation)?;
    let image = transform(image)?;
    check_tiff_cancellation(cancellation)?;
    writer.write_image(image, has_next, dpi)
}

fn check_tiff_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    crate::operation::check_cancellation(cancellation, "TIFF publication cancelled")
}

pub(super) fn write_multipage_tiff_rgb(
    frames: &[image::RgbImage],
    out: &Path,
    dpi: Option<u32>,
) -> Result<()> {
    validate_rgb_tiff_frames(frames)?;
    let temp = create_output_temp(out)?;
    let mut writer = TiffStreamWriter::new(temp.path())?;
    write_rgb_tiff_frames(&mut writer, frames, dpi)?;
    writer.finish()?;
    drop(writer);
    validate_output_container(temp.path(), "tiff")?;
    temp.publish()?;
    Ok(())
}

fn validate_rgb_tiff_frames(frames: &[image::RgbImage]) -> Result<()> {
    if frames.is_empty() {
        Err(ScanError::Invalid(
            "write_multipage_tiff_rgb requires at least one page".into(),
        ))
    } else {
        Ok(())
    }
}
fn write_rgb_tiff_frames(
    writer: &mut TiffStreamWriter,
    frames: &[image::RgbImage],
    dpi: Option<u32>,
) -> Result<()> {
    for (index, frame) in frames.iter().enumerate() {
        writer.write_rgb_frame(frame, index + 1 < frames.len(), dpi)?;
    }
    Ok(())
}

/// A classic-TIFF writer that retains only its current page. `next_ifd_at`
/// tracks the aggregate document layout, so every page is checked against the
/// 32-bit TIFF offset limit before RGB conversion or file writes.
struct TiffStreamWriter {
    file: std::fs::File,
    next_ifd_at: u32,
    previous_next_ifd_field_at: Option<u32>,
}

impl TiffStreamWriter {
    fn new(path: &Path) -> Result<Self> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)?;
        file.write_all(b"II")?;
        file.write_all(&42u16.to_le_bytes())?;
        file.write_all(&TIFF_HEADER_SIZE.to_le_bytes())?;
        Ok(Self {
            file,
            next_ifd_at: TIFF_HEADER_SIZE,
            previous_next_ifd_field_at: None,
        })
    }

    fn write_image(
        &mut self,
        image: ImageBuffer,
        has_next_page: bool,
        dpi: Option<u32>,
    ) -> Result<()> {
        let layout = self.preflight_page(image.width, image.height, has_next_page)?;
        let frame = image_buffer_to_rgb(image)?;
        self.write_planned_page(&frame, layout, dpi)
    }

    fn write_rgb_frame(
        &mut self,
        frame: &image::RgbImage,
        has_next_page: bool,
        dpi: Option<u32>,
    ) -> Result<()> {
        let layout = self.preflight_page(frame.width(), frame.height(), has_next_page)?;
        self.write_planned_page(frame, layout, dpi)
    }

    fn preflight_page(
        &self,
        width: u32,
        height: u32,
        has_next_page: bool,
    ) -> Result<TiffPageLayout> {
        let rgb_len =
            crate::domain::image::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
        let strip_len = u32::try_from(rgb_len).map_err(|_| {
            ScanError::Image("TIFF strip data exceeds classic TIFF size limit".into())
        })?;
        TiffPageLayout::new(self.next_ifd_at, strip_len, has_next_page)
    }

    fn write_planned_page(
        &mut self,
        frame: &image::RgbImage,
        layout: TiffPageLayout,
        dpi: Option<u32>,
    ) -> Result<()> {
        let expected_len = usize::try_from(layout.strip_len).map_err(|_| {
            ScanError::Image("TIFF strip size cannot be represented on this platform".into())
        })?;
        if frame.as_raw().len() != expected_len {
            return Err(ScanError::Image("invalid TIFF RGB frame buffer".into()));
        }
        if let Some(previous_next_ifd_field_at) = self.previous_next_ifd_field_at {
            self.file
                .seek(SeekFrom::Start(u64::from(previous_next_ifd_field_at)))?;
            self.file.write_all(&layout.ifd_at.to_le_bytes())?;
        }
        self.file.seek(SeekFrom::Start(u64::from(layout.ifd_at)))?;
        let resolution = dpi.unwrap_or(72).max(1);
        write_tiff_page(&mut self.file, frame, &layout, resolution, resolution)?;
        self.next_ifd_at = layout.next_ifd_at;
        self.previous_next_ifd_field_at = Some(layout.next_ifd_field_at);
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

fn image_buffer_to_rgb(image: ImageBuffer) -> Result<image::RgbImage> {
    super::pixels::into_rgb_image(image, "TIFF")
}

fn write_tiff_page(
    output: &mut impl Write,
    frame: &image::RgbImage,
    layout: &TiffPageLayout,
    xres: u32,
    yres: u32,
) -> Result<()> {
    write_tiff_ifd(output, frame, layout)?;
    write_rgb_bits_per_sample(output)?;
    write_tiff_rational(output, xres, 1)?;
    write_tiff_rational(output, yres, 1)?;
    output.write_all(frame.as_raw())?;
    if layout.next_ifd_at != 0 {
        output.write_all(&[0])?;
    }
    Ok(())
}

fn write_tiff_ifd(
    output: &mut impl Write,
    frame: &image::RgbImage,
    layout: &TiffPageLayout,
) -> Result<()> {
    let width = frame.width();
    let height = frame.height();
    let mut ifd = Vec::with_capacity(TIFF_IFD_SIZE as usize);
    ifd.extend_from_slice(&TIFF_IFD_ENTRY_COUNT.to_le_bytes());
    ifd.extend_from_slice(&tiff_entry(256, 4, 1, width));
    ifd.extend_from_slice(&tiff_entry(257, 4, 1, height));
    ifd.extend_from_slice(&tiff_entry(258, 3, 3, layout.bps_at));
    ifd.extend_from_slice(&tiff_entry(259, 3, 1, 1));
    ifd.extend_from_slice(&tiff_entry(262, 3, 1, 2));
    ifd.extend_from_slice(&tiff_entry(273, 4, 1, layout.strip_at));
    ifd.extend_from_slice(&tiff_entry(277, 3, 1, 3));
    ifd.extend_from_slice(&tiff_entry(278, 4, 1, height));
    ifd.extend_from_slice(&tiff_entry(279, 4, 1, layout.strip_len));
    ifd.extend_from_slice(&tiff_entry(282, 5, 1, layout.xres_at));
    ifd.extend_from_slice(&tiff_entry(283, 5, 1, layout.yres_at));
    ifd.extend_from_slice(&tiff_entry(296, 3, 1, 2));
    ifd.extend_from_slice(&0u32.to_le_bytes());
    output.write_all(&ifd)?;
    Ok(())
}

fn tiff_entry(tag: u16, typ: u16, count: u32, value: u32) -> [u8; 12] {
    let mut entry = [0u8; 12];
    entry[0..2].copy_from_slice(&tag.to_le_bytes());
    entry[2..4].copy_from_slice(&typ.to_le_bytes());
    entry[4..8].copy_from_slice(&count.to_le_bytes());
    entry[8..12].copy_from_slice(&value.to_le_bytes());
    entry
}

fn write_rgb_bits_per_sample(output: &mut impl Write) -> Result<()> {
    let mut bits = [0u8; 6];
    for (index, value) in [8u16, 8, 8].iter().enumerate() {
        bits[index * 2..index * 2 + 2].copy_from_slice(&value.to_le_bytes());
    }
    output.write_all(&bits)?;
    Ok(())
}

fn write_tiff_rational(output: &mut impl Write, numerator: u32, denominator: u32) -> Result<()> {
    let mut rational = [0u8; 8];
    rational[0..4].copy_from_slice(&numerator.to_le_bytes());
    rational[4..8].copy_from_slice(&denominator.to_le_bytes());
    output.write_all(&rational)?;
    Ok(())
}
