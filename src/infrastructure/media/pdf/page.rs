use super::searchable::{encode_searchable_text, SearchableFont};
use crate::domain::image::{ImageBuffer, MAX_IMAGE_BYTES};
use crate::error::{Result, ScanError};
use crate::infrastructure::media::to_rgb_bytes;
use crate::operation::CancellationToken;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, ObjectId, Stream, StringFormat};

const MAX_PDF_RETAINED_IMAGE_STREAM_BYTES: usize = MAX_IMAGE_BYTES / 2;

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
        let image_stream = build_image_stream(image.width, image.height, to_rgb_bytes(image)?)?;
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
        check_pdf_cancellation(cancellation)?;
        super::super::codecs::validate_packed_buffer(&image)?;
        let dimensions = PdfPageDimensions::from_image(&image, self.context.dpi);
        let (width, height) = (image.width, image.height);
        let image_stream =
            build_image_stream(width, height, super::super::pixels::into_rgb_bytes(image)?)?;
        self.add_page_stream(
            index,
            dimensions,
            image_stream,
            searchable_text,
            cancellation,
        )
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

fn build_image_stream(width: u32, height: u32, rgb: Vec<u8>) -> Result<Stream> {
    let mut stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => i64::from(width),
            "Height" => i64::from(height),
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        },
        rgb,
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

fn check_pdf_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("PDF publication cancelled".into()));
    }
    Ok(())
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
            Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 10.into()]),
            // A transparent fill keeps text searchable in readers that omit
            // render-mode-3 glyphs from extraction, while preserving the scan
            // image as the only visible page content.
            Operation::new("gs", vec![Object::Name(b"GS1".to_vec())]),
            Operation::new("Tr", vec![0.into()]),
            // Keep the invisible text inside even very small PDF pages so extractors do not
            // discard it as clipped content.
            Operation::new("Td", vec![1.into(), 1.into()]),
            Operation::new(
                "Tj",
                vec![Object::String(
                    encode_searchable_text(text, font)?,
                    StringFormat::Hexadecimal,
                )],
            ),
            Operation::new("ET", vec![]),
        ]);
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
