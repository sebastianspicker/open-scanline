use super::codecs::{
    create_output_temp, validate_output_container, validate_packed_buffer, OutputTemp,
};
use super::load_image;
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use tiff::encoder::colortype::{ColorType, Gray8, RGB8, RGBA8};
use tiff::encoder::{
    Compression, DeflateLevel, Rational, TiffEncoder, TiffKind, TiffKindBig, TiffKindStandard,
};
use tiff::tags::ResolutionUnit;

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
    save_multipage_tiff_with_loader_and_transform(
        pages,
        out.as_ref(),
        dpi,
        |path| load_image(path),
        Ok,
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
    temp: OutputTemp,
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
    let mut writer = BufWriter::new(
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(temp.path())?,
    );
    let dpi = Some(dpi.unwrap_or(72));
    if needs_bigtiff(pages) {
        write_tiff_pages::<TiffKindBig, _, _>(
            &mut writer,
            pages,
            dpi,
            load,
            transform,
            cancellation,
        )?;
    } else {
        write_tiff_pages::<TiffKindStandard, _, _>(
            &mut writer,
            pages,
            dpi,
            load,
            transform,
            cancellation,
        )?;
    }
    writer.flush()?;
    drop(writer);
    validate_output_container(temp.path(), "tiff")?;
    check_tiff_cancellation(cancellation)?;
    temp.publish()
}

/// Classic TIFF offsets are 32-bit. Sum the uncompressed size of every page
/// (assumed RGB, from the file header) and use BigTIFF when it would not fit.
/// shortcut: a transform that enlarges pages is not predicted; the encoder
/// then fails with an offset error instead of switching formats.
fn needs_bigtiff(pages: &[PathBuf]) -> bool {
    let total = pages.iter().fold(0u64, |total, page| {
        let (width, height) = image::image_dimensions(page).unwrap_or((0, 0));
        total.saturating_add(u64::from(width) * u64::from(height) * 3)
    });
    exceeds_classic_tiff(total)
}

/// Headroom for incompressible strips, IFDs and strip tables.
fn exceeds_classic_tiff(total_bytes: u64) -> bool {
    total_bytes > u64::from(u32::MAX) / 16 * 15
}

fn write_tiff_pages<K: TiffKind, L, F>(
    writer: &mut BufWriter<File>,
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
    let mut encoder = new_encoder::<K>(writer)?;
    for path in pages {
        check_tiff_cancellation(cancellation)?;
        let image = load(path)?;
        check_tiff_cancellation(cancellation)?;
        let image = transform(image)?;
        check_tiff_cancellation(cancellation)?;
        write_tiff_page(&mut encoder, &image, dpi)?;
    }
    Ok(())
}

fn check_tiff_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    crate::operation::check_cancellation(cancellation, "TIFF publication cancelled")
}

/// Write one image as a single-page TIFF into an already reserved output file.
pub(super) fn write_single_tiff(
    image: &ImageBuffer,
    output: &Path,
    dpi: Option<u32>,
) -> Result<()> {
    let mut writer = BufWriter::new(OpenOptions::new().write(true).truncate(true).open(output)?);
    let mut encoder = new_encoder::<TiffKindStandard>(&mut writer)?;
    write_tiff_page(&mut encoder, image, dpi)?;
    writer.flush()?;
    Ok(())
}

fn new_encoder<K: TiffKind>(
    writer: &mut BufWriter<File>,
) -> Result<TiffEncoder<&mut BufWriter<File>, K>> {
    Ok(TiffEncoder::<_, K>::new_generic(writer)
        .map_err(tiff_error)?
        .with_compression(Compression::Deflate(DeflateLevel::Balanced)))
}

fn tiff_error(error: tiff::TiffError) -> ScanError {
    ScanError::Image(format!("TIFF encode failed: {error}"))
}

/// Grey stays one sample per pixel (BlackIsZero); everything else is RGB(A).
fn write_tiff_page<W: Write + Seek, K: TiffKind>(
    encoder: &mut TiffEncoder<W, K>,
    image: &ImageBuffer,
    dpi: Option<u32>,
) -> Result<()> {
    validate_packed_buffer(image)?;
    match image.pixel_format {
        PixelFormat::Gray8 => write_typed_page::<Gray8, _, _>(encoder, image, dpi),
        PixelFormat::Rgb8 => write_typed_page::<RGB8, _, _>(encoder, image, dpi),
        PixelFormat::Rgba8 => write_typed_page::<RGBA8, _, _>(encoder, image, dpi),
    }
}

fn write_typed_page<C: ColorType<Inner = u8>, W: Write + Seek, K: TiffKind>(
    encoder: &mut TiffEncoder<W, K>,
    image: &ImageBuffer,
    dpi: Option<u32>,
) -> Result<()>
where
    [u8]: tiff::encoder::TiffValue,
{
    let len = image.width as usize * image.height as usize * image.bpp();
    let mut page = encoder
        .new_image::<C>(image.width, image.height)
        .map_err(tiff_error)?;
    if let Some(dpi) = dpi {
        let value = Rational {
            n: dpi.max(1),
            d: 1,
        };
        // `ImageEncoder::resolution` unwraps write errors; propagate them instead.
        let tags = page.encoder();
        tags.write_tag(
            tiff::tags::Tag::ResolutionUnit,
            ResolutionUnit::Inch.to_u16(),
        )
        .map_err(tiff_error)?;
        tags.write_tag(tiff::tags::Tag::XResolution, value.clone())
            .map_err(tiff_error)?;
        tags.write_tag(tiff::tags::Tag::YResolution, value)
            .map_err(tiff_error)?;
    }
    page.write_data(&image.data[..len]).map_err(tiff_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::media::save_image;
    use crate::infrastructure::runtime::TemporaryOutput;
    use tiff::decoder::{ifd::Value, Decoder};
    use tiff::tags::Tag;

    #[test]
    fn bigtiff_threshold_is_fifteen_sixteenths_of_four_gib() {
        let threshold = u64::from(u32::MAX) / 16 * 15;
        assert!(!exceeds_classic_tiff(threshold));
        assert!(exceeds_classic_tiff(threshold + 1));
        assert!(!needs_bigtiff(&[]));
    }

    #[test]
    fn multipage_round_trip_keeps_grey_colour_and_resolution() {
        let dir = TemporaryOutput::new("tiff-test", "tif").unwrap();
        let sizes = [(5u32, 3u32, true), (7, 2, false), (3, 5, false)];
        let mut paths = Vec::new();
        for (index, (w, h, grey)) in sizes.into_iter().enumerate() {
            let (format, bpp) = if grey {
                (PixelFormat::Gray8, 1)
            } else {
                (PixelFormat::Rgb8, 3)
            };
            let data = (0..w * h * bpp).map(|v| (v % 251) as u8).collect();
            let image = ImageBuffer::new(w, h, format, data).unwrap();
            let path = dir.directory().join(format!("page{index}.png"));
            save_image(&path, &image, None, None).unwrap();
            paths.push(path);
        }
        let out = dir.directory().join("out.tif");
        save_multipage_tiff(&paths, &out, Some(300)).unwrap();

        let mut decoder = Decoder::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut seen = Vec::new();
        loop {
            seen.push((
                decoder.dimensions().unwrap(),
                decoder.colortype().unwrap(),
                decoder.get_tag(Tag::XResolution).unwrap(),
                decoder.get_tag(Tag::YResolution).unwrap(),
                decoder.get_tag_u32(Tag::ResolutionUnit).unwrap(),
            ));
            if !decoder.more_images() {
                break;
            }
            decoder.next_image().unwrap();
        }
        let dpi = Value::Rational(300, 1);
        let expected = |size, color| (size, color, dpi.clone(), dpi.clone(), 2);
        assert_eq!(
            seen,
            vec![
                expected((5, 3), tiff::ColorType::Gray(8)),
                expected((7, 2), tiff::ColorType::RGB(8)),
                expected((3, 5), tiff::ColorType::RGB(8)),
            ]
        );
    }
}
