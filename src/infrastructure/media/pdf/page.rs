use super::check_pdf_cancellation;
use super::searchable::{
    encode_searchable_text, searchable_line_width, SearchableFont, SEARCHABLE_FONT_SIZE,
};
use crate::domain::image::{ImageBuffer, PixelFormat, MAX_IMAGE_BYTES};
use crate::error::{Result, ScanError};
use crate::infrastructure::media::codecs::validate_packed_buffer;
use crate::infrastructure::media::to_rgb_bytes;
use crate::operation::CancellationToken;
use image::codecs::jpeg::JpegEncoder;
use image::ExtendedColorType;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};

const MAX_PDF_RETAINED_IMAGE_STREAM_BYTES: usize = MAX_IMAGE_BYTES / 2;
const PDF_JPEG_QUALITY: u8 = 90;

pub(super) struct PdfPageContext {
    pub(super) pages_id: ObjectId,
    pub(super) searchable_font: Option<SearchableFont>,
    pub(super) dpi: f64,
}

pub(super) struct PdfPageBuilder<'a> {
    pub(super) document: &'a mut Document,
    pub(super) context: PdfPageContext,
    pub(super) retained_image_stream_bytes: usize,
}

impl<'a> PdfPageBuilder<'a> {
    pub(super) fn add_page(
        &mut self,
        index: usize,
        image: &ImageBuffer,
        searchable_text: Option<&str>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ObjectId> {
        check_pdf_cancellation(cancellation)?;
        let dimensions = PdfPageDimensions::from_image(image, self.context.dpi);
        let image_stream = build_image_stream(image)?;
        self.add_page_stream(
            index,
            dimensions,
            image_stream,
            searchable_text,
            cancellation,
        )
    }

    pub(super) fn add_page_owned(
        &mut self,
        index: usize,
        image: ImageBuffer,
        searchable_text: Option<&str>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ObjectId> {
        self.add_page(index, &image, searchable_text, cancellation)
    }

    fn add_page_stream(
        &mut self,
        index: usize,
        dimensions: PdfPageDimensions,
        image_stream: Stream,
        searchable_text: Option<&str>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ObjectId> {
        self.retained_image_stream_bytes = checked_pdf_image_stream_aggregate(
            self.retained_image_stream_bytes,
            image_stream.content.len(),
        )?;
        check_pdf_cancellation(cancellation)?;
        let image_id = self.document.add_object(image_stream);
        let image_name = format!("Im{index}");
        let content_id =
            self.add_content_stream(&image_name, dimensions, searchable_text, cancellation)?;
        let resources = page_resources(
            &image_name,
            image_id,
            self.context
                .searchable_font
                .as_ref()
                .map(|font| font.object_id),
        );
        check_pdf_cancellation(cancellation)?;
        Ok(self.document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => self.context.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(dimensions.width), Object::Real(dimensions.height)],
            "Resources" => resources,
            "Contents" => content_id,
        }))
    }

    fn add_content_stream(
        &mut self,
        image_name: &str,
        dimensions: PdfPageDimensions,
        searchable_text: Option<&str>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ObjectId> {
        let content = Content {
            operations: page_operations(
                image_name,
                dimensions,
                searchable_text,
                self.context.searchable_font.as_ref(),
            )?,
        }
        .encode()
        .map_err(|error| ScanError::Image(format!("PDF content encode failed: {error}")))?;
        let mut stream = Stream::new(dictionary! {}, content);
        stream.compress().map_err(|error| {
            ScanError::Image(format!("PDF content compression failed: {error}"))
        })?;
        check_pdf_cancellation(cancellation)?;
        Ok(self.document.add_object(stream))
    }
}

/// Grey pages stay `DeviceGray` (Flate). Colour pages are stored as JPEG
/// (`DCTDecode`); lopdf's `compress()` skips streams that already have a Filter.
fn build_image_stream(image: &ImageBuffer) -> Result<Stream> {
    validate_packed_buffer(image)?;
    let (width, height) = (image.width, image.height);
    let pixels = width as usize * height as usize;
    if image.pixel_format == PixelFormat::Gray8 {
        return flate_image_stream(width, height, "DeviceGray", image.data[..pixels].to_vec());
    }
    let rgb = if image.pixel_format == PixelFormat::Rgb8 {
        std::borrow::Cow::Borrowed(&image.data[..pixels * 3])
    } else {
        std::borrow::Cow::Owned(to_rgb_bytes(image)?)
    };
    let mut jpeg = Vec::new();
    let encoded = JpegEncoder::new_with_quality(&mut jpeg, PDF_JPEG_QUALITY).encode(
        &rgb,
        width,
        height,
        ExtendedColorType::Rgb8,
    );
    if encoded.is_err() {
        // JPEG cannot hold dimensions above 65535; fall back to lossless Flate.
        return flate_image_stream(width, height, "DeviceRGB", rgb.into_owned());
    }
    Ok(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
            "Filter" => "DCTDecode",
        },
        jpeg,
    ))
}

fn flate_image_stream(width: u32, height: u32, color_space: &str, data: Vec<u8>) -> Result<Stream> {
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => color_space,
            "BitsPerComponent" => 8,
        },
        data,
    );
    stream
        .compress()
        .map_err(|error| ScanError::Image(format!("PDF image compression failed: {error}")))?;
    Ok(stream)
}

fn checked_pdf_image_stream_aggregate(current: usize, stream_bytes: usize) -> Result<usize> {
    let aggregate = current.checked_add(stream_bytes).ok_or_else(|| {
        ScanError::Image("PDF image stream aggregate exceeds the safety limit".into())
    })?;
    if aggregate > MAX_PDF_RETAINED_IMAGE_STREAM_BYTES {
        return Err(ScanError::Image(format!(
            "PDF image stream aggregate exceeds the {MAX_PDF_RETAINED_IMAGE_STREAM_BYTES}-byte safety limit"
        )));
    }
    Ok(aggregate)
}

#[derive(Clone, Copy)]
struct PdfPageDimensions {
    width: f32,
    height: f32,
}

impl PdfPageDimensions {
    fn from_image(image: &ImageBuffer, dpi: f64) -> Self {
        Self {
            width: (image.width as f64 * 72.0 / dpi) as f32,
            height: (image.height as f64 * 72.0 / dpi) as f32,
        }
    }
}

/// Write one page per image with library-managed objects, streams, metadata and encryption.
fn page_operations(
    image_name: &str,
    dimensions: PdfPageDimensions,
    searchable_text: Option<&str>,
    searchable_font: Option<&SearchableFont>,
) -> Result<Vec<Operation>> {
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                Object::Real(dimensions.width),
                0.into(),
                0.into(),
                Object::Real(dimensions.height),
                0.into(),
                0.into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(image_name.as_bytes().to_vec())]),
        Operation::new("Q", vec![]),
    ];
    if let Some(text) = searchable_text {
        let font = searchable_font.ok_or_else(|| {
            ScanError::Other("searchable PDF text was supplied without a Type0 font".into())
        })?;
        operations.extend([
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![Object::Name(b"F1".to_vec()), SEARCHABLE_FONT_SIZE.into()],
            ),
            // A transparent fill keeps text searchable in readers that omit
            // render-mode-3 glyphs from extraction, while preserving the scan
            // image as the only visible page content.
            Operation::new("gs", vec![Object::Name(b"GS1".to_vec())]),
            Operation::new("Tr", vec![0.into()]),
        ]);
        operations.extend(searchable_line_operations(text, font, dimensions)?);
        operations.push(Operation::new("ET", vec![]));
    }
    Ok(operations)
}

/// Lay OCR text out as one positioned run per line, from the top of the page
/// downwards. Glyph advances come from the font's width table, and a line
/// wider than the page is squeezed with `Tz` so it stays on the page. That
/// keeps words and spaces on one text line for extractors such as `pdftotext`.
fn searchable_line_operations(
    text: &str,
    font: &SearchableFont,
    dimensions: PdfPageDimensions,
) -> Result<Vec<Operation>> {
    // Keep the invisible text inside even very small PDF pages so extractors
    // do not discard it as clipped content.
    const MARGIN: f32 = 1.0;
    let lines = text.split('\n').collect::<Vec<_>>();
    let available_height = (dimensions.height - 2.0 * MARGIN).max(1.0);
    let leading = (SEARCHABLE_FONT_SIZE as f32 * 1.2).min(available_height / lines.len() as f32);
    let available_width = (dimensions.width - 2.0 * MARGIN).max(1.0);
    let mut operations = Vec::new();
    let mut previous_scale = 100.0_f32;
    let mut y_step = (dimensions.height - MARGIN - leading).max(MARGIN);
    let mut x_step = MARGIN;
    for line in lines {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.is_empty() {
            let natural_width = searchable_line_width(line, f32::from(SEARCHABLE_FONT_SIZE));
            let scale = if natural_width > available_width {
                available_width / natural_width * 100.0
            } else {
                100.0
            };
            if scale != previous_scale {
                operations.push(Operation::new("Tz", vec![Object::Real(scale)]));
                previous_scale = scale;
            }
            operations.push(Operation::new(
                "Td",
                vec![Object::Real(x_step), Object::Real(y_step)],
            ));
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(
                    encode_searchable_text(line, font)?,
                    StringFormat::Hexadecimal,
                )],
            ));
            // `Td` is relative to the start of the previous line.
            x_step = 0.0;
            y_step = -leading;
        } else {
            y_step -= leading;
        }
    }
    Ok(operations)
}

fn page_resources(
    image_name: &str,
    image_id: ObjectId,
    font_id: Option<ObjectId>,
) -> lopdf::Dictionary {
    let mut xobjects = lopdf::Dictionary::new();
    xobjects.set(image_name.as_bytes(), image_id);
    let mut resources = dictionary! { "XObject" => xobjects };
    if let Some(font) = font_id {
        resources.set("Font", dictionary! { "F1" => font });
        resources.set(
            "ExtGState",
            dictionary! {
                "GS1" => dictionary! {
                    "Type" => "ExtGState",
                    "ca" => Object::Real(0.01),
                    "CA" => Object::Real(0.01),
                },
            },
        );
    }
    resources
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn font_for(text: &str) -> SearchableFont {
        let mut characters = text.chars().collect::<Vec<_>>();
        characters.sort_unstable();
        characters.dedup();
        SearchableFont {
            object_id: (1, 0),
            character_codes: characters
                .into_iter()
                .enumerate()
                .map(|(index, character)| (character, (index + 1) as u16))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    fn text_operations(text: &str, width: f32, height: f32) -> Vec<Operation> {
        let font = font_for(text);
        let dimensions = PdfPageDimensions { width, height };
        page_operations("Im0", dimensions, Some(text), Some(&font)).unwrap()
    }

    fn image_filters(path: &std::path::Path, password: Option<&str>) -> Vec<(String, String)> {
        let document = match password {
            Some(password) => Document::load_with_password(path, password).unwrap(),
            None => Document::load(path).unwrap(),
        };
        assert_eq!(document.get_pages().len(), 2);
        let mut found = Vec::new();
        for object in document.objects.values() {
            let Object::Stream(stream) = object else {
                continue;
            };
            if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image") {
                continue;
            }
            let name = |key: &[u8]| {
                String::from_utf8_lossy(stream.dict.get(key).unwrap().as_name().unwrap())
                    .into_owned()
            };
            found.push((name(b"ColorSpace"), name(b"Filter")));
        }
        found.sort();
        found
    }

    #[test]
    fn grey_pages_stay_grey_and_colour_pages_are_jpeg() {
        let pages = [
            ImageBuffer::new(32, 32, PixelFormat::Gray8, vec![128; 32 * 32]).unwrap(),
            ImageBuffer::new(32, 32, PixelFormat::Rgb8, vec![200; 32 * 32 * 3]).unwrap(),
        ];
        let dir = crate::infrastructure::runtime::TemporaryOutput::new("pdf-test", "pdf").unwrap();
        let expected = vec![
            ("DeviceGray".to_string(), "FlateDecode".to_string()),
            ("DeviceRGB".to_string(), "DCTDecode".to_string()),
        ];

        let plain = dir.directory().join("plain.pdf");
        crate::infrastructure::media::save_pdf_with_options(
            &plain,
            &pages,
            &crate::infrastructure::media::PdfOptions::default(),
        )
        .unwrap();
        assert_eq!(image_filters(&plain, None), expected);

        let locked = dir.directory().join("locked.pdf");
        crate::infrastructure::media::save_pdf_with_options(&locked, &pages, &{
            let mut options = crate::infrastructure::media::PdfOptions::default();
            options.password = Some("secret".into());
            options
        })
        .unwrap();
        assert_eq!(image_filters(&locked, Some("secret")), expected);
    }

    #[test]
    fn each_line_is_one_positioned_run_inside_the_page() {
        let operations = text_operations("HELLO WORLD\nSECOND LINE\n\nLAST", 612.0, 792.0);
        let runs = operations.iter().filter(|op| op.operator == "Tj").count();
        assert_eq!(runs, 3);
        let moves = operations
            .iter()
            .filter(|op| op.operator == "Td")
            .map(|op| {
                op.operands
                    .iter()
                    .map(|value| value.as_float().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(moves.len(), 3);
        // The first line starts near the top; later lines step down, with the
        // blank line consuming one extra step.
        assert!(moves[0][1] > 700.0 && moves[0][1] < 792.0);
        assert_eq!(moves[1], vec![0.0, -12.0]);
        assert_eq!(moves[2], vec![0.0, -24.0]);
        assert!(operations.iter().all(|op| op.operator != "Tz"));
    }

    #[test]
    fn long_lines_are_squeezed_to_the_page_width() {
        let text = "WORD ".repeat(40);
        let line = text.trim_end();
        let operations = text_operations(line, 200.0, 100.0);
        let scale = operations
            .iter()
            .find(|op| op.operator == "Tz")
            .and_then(|op| op.operands[0].as_float().ok())
            .expect("long line sets a horizontal scale");
        let natural = searchable_line_width(line, f32::from(SEARCHABLE_FONT_SIZE));
        assert!(natural > 198.0);
        let squeezed = natural * scale / 100.0;
        assert!((squeezed - 198.0).abs() < 0.01, "{squeezed}");
    }

    #[test]
    fn spaces_are_narrower_than_other_glyphs() {
        let size = f32::from(SEARCHABLE_FONT_SIZE);
        assert!(searchable_line_width(" ", size) < searchable_line_width("A", size));
        assert_eq!(searchable_line_width("AB", size), 10.0);
    }
}
