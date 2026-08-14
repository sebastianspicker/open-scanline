use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};
use crate::device::CancellationToken;
use crate::imaging::codecs::{buffer_to_dynamic, create_output_temp, validate_output_container};
use crate::imaging::{load_image, save_image, save_pdf_with_options, PdfOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

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
        let bps_at = checked_tiff_offset(ifd_at, TIFF_IFD_SIZE)?;
        let xres_at = checked_tiff_offset(bps_at, 6)?;
        let yres_at = checked_tiff_offset(xres_at, 8)?;
        let strip_at = checked_tiff_offset(yres_at, 8)?;
        let strip_end = checked_tiff_offset(strip_at, strip_len)?;
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
    let frame_rgb_bytes = crate::core::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
    let aggregate_rgb_bytes = current_rgb_bytes
        .checked_add(frame_rgb_bytes)
        .ok_or_else(|| {
            ScanError::Image("TIFF decoded frame aggregate exceeds the image safety limit".into())
        })?;
    if aggregate_rgb_bytes > crate::core::MAX_IMAGE_BYTES {
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
    if pages.is_empty() {
        return Err(ScanError::Invalid(
            "save_multipage_tiff requires at least one page".into(),
        ));
    }
    check_tiff_cancellation(cancellation)?;
    let out = out.as_ref();
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let temp = create_output_temp(out)?;
    let mut writer = TiffStreamWriter::new(temp.path())?;
    for (page_index, path) in pages.iter().enumerate() {
        check_tiff_cancellation(cancellation)?;
        let buf = load_image(path)?;
        check_tiff_cancellation(cancellation)?;
        let buf = transform(&buf)?;
        check_tiff_cancellation(cancellation)?;
        writer.write_image(buf, page_index + 1 < pages.len(), dpi)?;
    }
    writer.finish()?;
    drop(writer);
    validate_output_container(temp.path(), "tiff")?;
    check_tiff_cancellation(cancellation)?;
    temp.publish()?;
    Ok(out.to_path_buf())
}

fn check_tiff_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("TIFF publication cancelled".into()));
    }
    Ok(())
}

pub(super) fn write_multipage_tiff_rgb(
    frames: &[image::RgbImage],
    out: &Path,
    dpi: Option<u32>,
) -> Result<()> {
    if frames.is_empty() {
        return Err(ScanError::Invalid(
            "write_multipage_tiff_rgb requires at least one page".into(),
        ));
    }
    let temp = create_output_temp(out)?;
    let mut writer = TiffStreamWriter::new(temp.path())?;
    for (page_index, frame) in frames.iter().enumerate() {
        writer.write_rgb_frame(frame, page_index + 1 < frames.len(), dpi)?;
    }
    writer.finish()?;
    drop(writer);
    validate_output_container(temp.path(), "tiff")?;
    temp.publish()?;
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
        let rgb_len = crate::core::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
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
    match image.pixel_format {
        PixelFormat::Rgb8 => image::RgbImage::from_raw(image.width, image.height, image.data)
            .ok_or_else(|| ScanError::Image("invalid TIFF RGB image buffer".into())),
        PixelFormat::Gray8 | PixelFormat::Rgba8 => Ok(buffer_to_dynamic(&image)?.to_rgb8()),
    }
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

/// Append a page to TIFF, or create/replace a PDF as in the legacy application workflow.
pub fn append_page_to_multipage(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    format: Option<&str>,
    dpi: Option<u32>,
) -> Result<PathBuf> {
    let mut out = path.as_ref().to_path_buf();
    let ext = format
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| {
            out.extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_else(|| "tif".into());
    match ext.as_str() {
        "tif" | "tiff" => {
            if out.extension().is_none() {
                out.set_extension("tif");
            }
            if !out.is_file() {
                return save_image(out, image, dpi, None);
            }
            let mut frames = load_tiff_frames(&out)?;
            let existing_rgb_bytes = frames.iter().try_fold(0usize, |total, frame| {
                total.checked_add(frame.as_raw().len()).ok_or_else(|| {
                    ScanError::Image(
                        "TIFF decoded frame aggregate exceeds the image safety limit".into(),
                    )
                })
            })?;
            checked_tiff_frame_aggregate(existing_rgb_bytes, image.width, image.height)?;
            frames.push(buffer_to_dynamic(image)?.to_rgb8());
            write_multipage_tiff_rgb(&frames, &out, dpi)?;
            Ok(out)
        }
        "pdf" => {
            if out.extension().is_none() {
                out.set_extension("pdf");
            }
            save_pdf_with_options(
                out,
                std::slice::from_ref(image),
                &PdfOptions {
                    dpi: dpi.unwrap_or(150),
                    ..PdfOptions::default()
                },
            )
        }
        _ => Err(ScanError::Invalid(format!(
            "append_page_to_multipage supports TIFF or PDF, not '{ext}'"
        ))),
    }
}

pub(super) fn load_tiff_frames(path: &Path) -> Result<Vec<image::RgbImage>> {
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::ColorType;

    let file = std::fs::File::open(path)?;
    let mut decoder = Decoder::new(std::io::BufReader::new(file))
        .map_err(|e| ScanError::Image(format!("TIFF decode failed: {e}")))?;
    let mut frames = Vec::new();
    let mut aggregate_rgb_bytes = 0usize;
    loop {
        let (width, height) = decoder
            .dimensions()
            .map_err(|e| ScanError::Image(format!("TIFF dimensions failed: {e}")))?;
        let color = decoder
            .colortype()
            .map_err(|e| ScanError::Image(format!("TIFF color type failed: {e}")))?;
        if !matches!(color, ColorType::RGB(8) | ColorType::Gray(8)) {
            return Err(ScanError::Image(
                "TIFF append supports 8-bit grayscale or RGB pages".into(),
            ));
        }
        aggregate_rgb_bytes = checked_tiff_frame_aggregate(aggregate_rgb_bytes, width, height)?;
        let bytes = match decoder
            .read_image()
            .map_err(|e| ScanError::Image(format!("TIFF frame decode failed: {e}")))?
        {
            DecodingResult::U8(data) => data,
            _ => {
                return Err(ScanError::Image(
                    "TIFF append supports 8-bit grayscale or RGB pages".into(),
                ));
            }
        };
        let rgb = match color {
            ColorType::RGB(8) => image::RgbImage::from_raw(width, height, bytes),
            ColorType::Gray(8) => {
                let data = bytes
                    .into_iter()
                    .flat_map(|value| [value, value, value])
                    .collect();
                image::RgbImage::from_raw(width, height, data)
            }
            _ => None,
        }
        .ok_or_else(|| ScanError::Image("invalid TIFF frame buffer".into()))?;
        frames.push(rgb);
        if !decoder.more_images() {
            break;
        }
        decoder
            .next_image()
            .map_err(|e| ScanError::Image(format!("TIFF next frame failed: {e}")))?;
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn rgb_multipage_writer_uses_aligned_ifds_and_preserves_page_pixels() {
        let path = std::env::temp_dir().join(format!(
            "open_scanline_tiff_layout_{}.tif",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let frames = vec![
            image::RgbImage::from_raw(1, 1, vec![1, 2, 3]).unwrap(),
            image::RgbImage::from_raw(2, 1, vec![4, 5, 6, 7, 8, 9]).unwrap(),
        ];

        write_multipage_tiff_rgb(&frames, &path, Some(300)).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"II\x2a\0");
        let mut ifd_at = read_u32(&bytes, 4);
        for (page_index, expected_next_link) in [true, false].into_iter().enumerate() {
            assert_eq!(ifd_at % TIFF_WORD_ALIGNMENT, 0);
            let (entries, next_ifd_at) = read_ifd(&bytes, ifd_at);
            assert_eq!(entries.len(), TIFF_IFD_ENTRY_COUNT as usize);
            assert_eq!(
                entries.iter().map(|(tag, _)| *tag).collect::<Vec<_>>(),
                vec![256, 257, 258, 259, 262, 273, 277, 278, 279, 282, 283, 296]
            );
            for tag in [258, 273, 282, 283] {
                assert_eq!(entry_value(&entries, tag) % TIFF_WORD_ALIGNMENT, 0);
            }
            for tag in [282, 283] {
                let offset = entry_value(&entries, tag);
                assert_eq!(read_u32(&bytes, offset), 300);
                assert_eq!(read_u32(&bytes, offset + 4), 1);
            }
            if expected_next_link {
                assert_ne!(next_ifd_at, 0);
                assert_eq!(next_ifd_at % TIFF_WORD_ALIGNMENT, 0);
                let strip_offset = entry_value(&entries, 273);
                let strip_len = entry_value(&entries, 279);
                assert_eq!(bytes[(strip_offset + strip_len) as usize], 0);
                ifd_at = next_ifd_at;
            } else {
                assert_eq!(next_ifd_at, 0, "page {page_index} must terminate the chain");
            }
        }
        assert_eq!(load_tiff_frames(&path).unwrap(), frames);
        assert_external_tiff_decoders(&path);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn layout_rejects_classic_tiff_offset_overflow_without_allocating() {
        let err = TiffPageLayout::new(u32::MAX - TIFF_IFD_SIZE + 1, 0, true).unwrap_err();
        assert!(matches!(
            err,
            ScanError::Image(message) if message == "TIFF layout exceeds classic TIFF offset limit"
        ));
    }

    #[test]
    fn frame_aggregate_limit_rejects_before_decoder_allocation() {
        let error = checked_tiff_frame_aggregate(crate::core::MAX_IMAGE_BYTES, 1, 1).unwrap_err();
        assert!(matches!(
            error,
            ScanError::Image(message)
                if message == "TIFF decoded frame aggregate exceeds the image safety limit"
        ));
    }

    #[test]
    fn writer_rejects_aggregate_classic_tiff_overflow_before_rgb_conversion() {
        let dir = test_dir("overflow");
        let destination = dir.join("output.tif");
        let temp = create_output_temp(&destination).unwrap();
        let temporary_path = temp.path().to_path_buf();
        let mut writer = TiffStreamWriter::new(temp.path()).unwrap();
        writer.next_ifd_at = u32::MAX - TIFF_IFD_SIZE + 1;

        let error = writer
            .write_image(
                ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![7]).unwrap(),
                true,
                None,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            ScanError::Image(message) if message == "TIFF layout exceeds classic TIFF offset limit"
        ));
        assert_eq!(
            std::fs::metadata(temp.path()).unwrap().len(),
            u64::from(TIFF_HEADER_SIZE)
        );
        drop(writer);
        drop(temp);
        assert!(!temporary_path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn path_writer_streams_many_pages_before_transforming_the_next_page() {
        let dir = test_dir("streaming");
        let destination = dir.join("output.tif");
        let pages = (0..48u8)
            .map(|index| {
                let path = dir.join(format!("page-{index:02}.png"));
                let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![index, 2, 3]).unwrap();
                save_image(&path, &image, None, None).unwrap();
                path
            })
            .collect::<Vec<_>>();
        let mut transformed_pages = 0usize;
        let mut observed_written_first_page = false;

        save_multipage_tiff_from_paths_with_transform(&pages, &destination, Some(300), |image| {
            transformed_pages += 1;
            if transformed_pages == 2 {
                observed_written_first_page = std::fs::read_dir(&dir)
                    .unwrap()
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .contains("open-scanline")
                    })
                    .any(|entry| entry.metadata().unwrap().len() > u64::from(TIFF_HEADER_SIZE));
            }
            Ok(image.clone())
        })
        .unwrap();

        assert_eq!(transformed_pages, pages.len());
        assert!(
            observed_written_first_page,
            "the first page must be written before the next page is transformed"
        );
        let frames = load_tiff_frames(&destination).unwrap();
        assert_eq!(frames.len(), pages.len());
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame.as_raw(), &[index as u8, 2, 3]);
        }
        assert_no_sibling_temps(&dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transform_failure_preserves_destination_and_cleans_streaming_temp() {
        let dir = test_dir("transform-failure");
        let destination = dir.join("output.tif");
        std::fs::write(&destination, b"previous TIFF output").unwrap();
        let pages = [dir.join("first.png"), dir.join("second.png")];
        for (index, path) in pages.iter().enumerate() {
            let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![index as u8, 2, 3]).unwrap();
            save_image(path, &image, None, None).unwrap();
        }
        let mut transformed_pages = 0usize;

        let error = save_multipage_tiff_from_paths_with_transform(
            &pages,
            &destination,
            Some(300),
            |image| {
                transformed_pages += 1;
                if transformed_pages == 2 {
                    return Err(ScanError::Image("transform failed".into()));
                }
                Ok(image.clone())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("transform failed"));
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"previous TIFF output"
        );
        assert_no_sibling_temps(&dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancellation_after_first_transform_preserves_existing_tiff() {
        let dir = test_dir("cancel-after-first");
        let destination = dir.join("output.tif");
        std::fs::write(&destination, b"previous TIFF output").unwrap();
        let pages = [dir.join("first.png"), dir.join("second.png")];
        for (index, path) in pages.iter().enumerate() {
            let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![index as u8, 2, 3]).unwrap();
            save_image(path, &image, None, None).unwrap();
        }
        let token = CancellationToken::new();
        let canceller = token.clone();
        let mut transforms = 0;

        let error = save_multipage_tiff_from_paths_with_transform_and_cancellation(
            &pages,
            &destination,
            Some(300),
            |image| {
                transforms += 1;
                if transforms == 1 {
                    canceller.cancel();
                }
                Ok(image.clone())
            },
            Some(&token),
        )
        .unwrap_err();

        assert!(matches!(error, ScanError::Cancelled(_)));
        assert_eq!(transforms, 1);
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"previous TIFF output"
        );
        assert_no_sibling_temps(&dir);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn multipage_writer_replaces_existing_output_after_validating_the_tiff() {
        let dir =
            std::env::temp_dir().join(format!("open_scanline_tiff_atomic_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("output.tif");
        std::fs::write(&path, b"previous TIFF output").unwrap();
        let frames = vec![image::RgbImage::from_raw(1, 1, vec![1, 2, 3]).unwrap()];

        write_multipage_tiff_rgb(&frames, &path, Some(300)).unwrap();

        assert_eq!(&std::fs::read(&path).unwrap()[..4], b"II\x2a\0");
        assert_eq!(load_tiff_frames(&path).unwrap(), frames);
        let temporary_count = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("open-scanline")
            })
            .count();
        assert_eq!(temporary_count, 0, "TIFF publication leaked a sibling temp");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn read_ifd(bytes: &[u8], ifd_at: u32) -> (Vec<(u16, u32)>, u32) {
        let start = ifd_at as usize;
        let count = read_u16(bytes, ifd_at) as usize;
        let entries = (0..count)
            .map(|index| {
                let at = start + 2 + index * 12;
                (read_u16(bytes, at as u32), read_u32(bytes, (at + 8) as u32))
            })
            .collect();
        let next_ifd_at = read_u32(bytes, (start + 2 + count * 12) as u32);
        (entries, next_ifd_at)
    }

    fn read_u16(bytes: &[u8], at: u32) -> u16 {
        u16::from_le_bytes(bytes[at as usize..at as usize + 2].try_into().unwrap())
    }

    fn read_u32(bytes: &[u8], at: u32) -> u32 {
        u32::from_le_bytes(bytes[at as usize..at as usize + 4].try_into().unwrap())
    }

    fn entry_value(entries: &[(u16, u32)], tag: u16) -> u32 {
        entries
            .iter()
            .find_map(|(candidate, value)| (*candidate == tag).then_some(*value))
            .unwrap()
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "open_scanline_tiff_{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn assert_no_sibling_temps(dir: &Path) {
        assert_eq!(
            std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains("open-scanline")
                })
                .count(),
            0,
            "TIFF export leaked a sibling temporary file"
        );
    }

    fn assert_external_tiff_decoders(path: &Path) {
        for command in ["tiffinfo", "identify"] {
            match Command::new(command).arg(path).output() {
                Ok(output) => assert!(
                    output.status.success(),
                    "{command} rejected generated TIFF: {}",
                    String::from_utf8_lossy(&output.stderr)
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("could not run {command}: {error}"),
            }
        }
    }
}
