use crate::core::{ImageBuffer, Result, ScanError, MAX_IMAGE_BYTES};
use crate::device::CancellationToken;
use crate::imaging::codecs::{create_output_temp, validate_output_container};
use crate::imaging::{load_image, to_rgb_bytes};
use lopdf::content::{Content, Operation};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    dictionary, Document, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions,
    Stream, StringFormat,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SEARCHABLE_FONT_BYTES: &[u8] = include_bytes!("../../assets/fonts/Cantarell-Regular.ttf");

/// PDF construction retains each compressed page stream until the document is
/// atomically published. Limit the aggregate compressed streams to half the
/// product's 512 MiB decoded-image ceiling, leaving room for the current page
/// and PDF object/compression overhead.
const MAX_PDF_RETAINED_IMAGE_STREAM_BYTES: usize = MAX_IMAGE_BYTES / 2;

/// OCR text is copied into the font map, CMap, and page content stream. A
/// 1 MiB page limit keeps one pathological OCR result from dominating those
/// allocations.
pub(crate) const MAX_PDF_SEARCHABLE_PAGE_UTF8_BYTES: usize = 1024 * 1024;
/// The aggregate searchable layer stays well below the retained 256 MiB image
/// stream budget, including for long feeder batches.
pub(crate) const MAX_PDF_SEARCHABLE_TEXT_UTF8_BYTES: usize = 16 * 1024 * 1024;

/// Options for structured PDF output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOptions {
    pub dpi: u32,
    pub title: String,
    pub password: Option<String>,
    pub searchable_pages: Option<Vec<String>>,
}

impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            dpi: 150,
            title: "open-scanline scan".into(),
            password: None,
            searchable_pages: None,
        }
    }
}

struct PdfPageContext {
    pages_id: ObjectId,
    searchable_font: Option<SearchableFont>,
    dpi: f64,
}

#[derive(Clone)]
struct SearchableFont {
    object_id: ObjectId,
    character_codes: BTreeMap<char, u16>,
}

struct PdfPageBuilder<'a> {
    document: &'a mut Document,
    context: PdfPageContext,
    retained_image_stream_bytes: usize,
}

impl<'a> PdfPageBuilder<'a> {
    fn add_page(
        &mut self,
        index: usize,
        image: &ImageBuffer,
        searchable_text: Option<&str>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<ObjectId> {
        check_pdf_cancellation(cancellation)?;
        let image_stream = self.build_image_stream(image)?;
        self.retained_image_stream_bytes = checked_pdf_image_stream_aggregate(
            self.retained_image_stream_bytes,
            image_stream.content.len(),
        )?;
        check_pdf_cancellation(cancellation)?;
        let image_id = self.document.add_object(image_stream);
        let dimensions = PdfPageDimensions::from_image(image, self.context.dpi);
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

    fn build_image_stream(&self, image: &ImageBuffer) -> Result<Stream> {
        let rgb = to_rgb_bytes(image)?;
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => i64::from(image.width),
                "Height" => i64::from(image.height),
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
pub fn save_pdf_with_options(
    path: impl AsRef<Path>,
    images: &[ImageBuffer],
    options: &PdfOptions,
) -> Result<PathBuf> {
    save_pdf_with_options_and_cancellation(path, images, options, None)
}

/// Write a PDF while allowing a shared operation token to stop publication.
pub fn save_pdf_with_options_and_cancellation(
    path: impl AsRef<Path>,
    images: &[ImageBuffer],
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    validate_pdf_input(images, options)?;
    check_pdf_cancellation(cancellation)?;
    let path = path.as_ref();
    create_pdf_parent(path)?;
    let (mut doc, pages_id, searchable_font) = create_pdf_document(options, cancellation)?;
    let mut page_ids = Vec::with_capacity(images.len());
    {
        let mut pages = PdfPageBuilder {
            document: &mut doc,
            context: PdfPageContext {
                pages_id,
                searchable_font,
                dpi: options.dpi.max(1) as f64,
            },
            retained_image_stream_bytes: 0,
        };
        for (index, image) in images.iter().enumerate() {
            check_pdf_cancellation(cancellation)?;
            let searchable_text = options
                .searchable_pages
                .as_ref()
                .map(|pages| pages[index].as_str());
            page_ids.push(pages.add_page(index, image, searchable_text, cancellation)?);
        }
    }
    finish_pdf_document(&mut doc, pages_id, &page_ids, path, options, cancellation)?;
    Ok(path.to_path_buf())
}

fn create_pdf_document(
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<(Document, ObjectId, Option<SearchableFont>)> {
    let mut document = Document::with_version(if options.password.is_some() {
        "2.0"
    } else {
        "1.5"
    });
    let pages_id = document.new_object_id();
    let searchable_font = add_searchable_font(
        &mut document,
        options.searchable_pages.as_deref(),
        cancellation,
    )?;
    Ok((document, pages_id, searchable_font))
}

fn finish_pdf_document(
    document: &mut Document,
    pages_id: ObjectId,
    page_ids: &[ObjectId],
    path: &Path,
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    configure_pdf_document(document, pages_id, page_ids, options, cancellation)?;
    check_pdf_cancellation(cancellation)?;
    encrypt_pdf_document(document, options)?;
    save_pdf_document(document, path, cancellation)
}

fn validate_pdf_input(images: &[ImageBuffer], options: &PdfOptions) -> Result<()> {
    if images.is_empty() {
        return Err(ScanError::Invalid(
            "save_pdf requires at least one page".into(),
        ));
    }
    validate_pdf_options(images.len(), options)
}

fn validate_pdf_options(page_count: usize, options: &PdfOptions) -> Result<()> {
    if let Some(text) = options.searchable_pages.as_ref() {
        if text.len() != page_count {
            return Err(ScanError::Invalid(
                "searchable_pages length must match number of pages".into(),
            ));
        }
        validate_pdf_searchable_pages(text)?;
    }
    if let Some(password) = options.password.as_deref() {
        if password.is_empty() {
            return Err(ScanError::Invalid(
                "PDF password must not be empty when encryption is requested".into(),
            ));
        }
        validate_pdf_password(password)?;
    }
    Ok(())
}

/// Validate searchable-PDF OCR text before it is copied into document objects.
/// Batch acquisition can also use this validator while accumulating OCR output.
pub(crate) fn validate_pdf_searchable_pages(searchable_pages: &[String]) -> Result<()> {
    let mut aggregate = 0;
    for (index, text) in searchable_pages.iter().enumerate() {
        aggregate = checked_pdf_searchable_text_total(aggregate, text, index)?;
    }
    Ok(())
}

/// Validate one OCR result before retaining it and return the new aggregate.
pub(crate) fn checked_pdf_searchable_text_total(
    aggregate: usize,
    text: &str,
    page_index: usize,
) -> Result<usize> {
    checked_pdf_searchable_text_total_with_limits(
        aggregate,
        text,
        page_index,
        MAX_PDF_SEARCHABLE_PAGE_UTF8_BYTES,
        MAX_PDF_SEARCHABLE_TEXT_UTF8_BYTES,
    )
}

fn checked_pdf_searchable_text_total_with_limits(
    aggregate: usize,
    text: &str,
    page_index: usize,
    page_limit: usize,
    aggregate_limit: usize,
) -> Result<usize> {
    if text.len() > page_limit {
        return Err(ScanError::Invalid(format!(
            "searchable PDF page {} exceeds the {page_limit}-byte UTF-8 safety limit",
            page_index + 1
        )));
    }
    let aggregate = aggregate.checked_add(text.len()).ok_or_else(|| {
        ScanError::Invalid("searchable PDF text aggregate exceeds the safety limit".into())
    })?;
    if aggregate > aggregate_limit {
        return Err(ScanError::Invalid(format!(
            "searchable PDF text aggregate exceeds the {aggregate_limit}-byte UTF-8 safety limit"
        )));
    }
    Ok(aggregate)
}

fn create_pdf_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

fn add_searchable_font(
    document: &mut Document,
    searchable_pages: Option<&[String]>,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<SearchableFont>> {
    let Some(searchable_pages) = searchable_pages else {
        return Ok(None);
    };

    let characters = searchable_pages
        .iter()
        .flat_map(|text| text.chars())
        .collect::<BTreeSet<_>>();
    if characters.len() > u16::MAX as usize {
        return Err(ScanError::Invalid(
            "searchable PDF text has more than 65,535 distinct Unicode characters".into(),
        ));
    }
    let character_codes = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| (character, (index + 1) as u16))
        .collect::<BTreeMap<_, _>>();
    check_pdf_cancellation(cancellation)?;
    let to_unicode_id = document.add_object(Stream::new(
        dictionary! {},
        searchable_to_unicode_cmap(&character_codes),
    ));
    check_pdf_cancellation(cancellation)?;
    let font_file_id = document.add_object(Stream::new(
        dictionary! {
            "Length1" => SEARCHABLE_FONT_BYTES.len() as i64,
            "Filter" => "Crypt",
            "DecodeParms" => dictionary! { "Name" => "Identity" },
        },
        SEARCHABLE_FONT_BYTES.to_vec(),
    ));
    check_pdf_cancellation(cancellation)?;
    let descriptor_id = document.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => "Cantarell-Regular",
        "Flags" => 4,
        "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()],
        "ItalicAngle" => 0,
        "Ascent" => 880,
        "Descent" => -120,
        "CapHeight" => 700,
        "StemV" => 80,
        "FontFile2" => font_file_id,
    });
    check_pdf_cancellation(cancellation)?;
    let cid_to_gid_id = document.add_object(Stream::new(
        dictionary! {},
        cid_to_gid_map(character_codes.len()),
    ));
    check_pdf_cancellation(cancellation)?;
    let cid_font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => "Cantarell-Regular",
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0,
        },
        "FontDescriptor" => descriptor_id,
        "DW" => 1000,
        "CIDToGIDMap" => cid_to_gid_id,
    });
    check_pdf_cancellation(cancellation)?;
    let object_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => "Cantarell-Regular",
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![cid_font_id.into()],
        "ToUnicode" => to_unicode_id,
    });
    Ok(Some(SearchableFont {
        object_id,
        character_codes,
    }))
}

fn cid_to_gid_map(character_count: usize) -> Vec<u8> {
    let mut map = Vec::with_capacity((character_count + 1) * 2);
    map.extend_from_slice(&0_u16.to_be_bytes());
    for _ in 0..character_count {
        // Map every CID to a valid, unremarkable glyph. The text is rendered
        // invisibly; ToUnicode supplies the extraction semantics.
        map.extend_from_slice(&1_u16.to_be_bytes());
    }
    map
}

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

fn encode_searchable_text(text: &str, font: &SearchableFont) -> Result<Vec<u8>> {
    let mut encoded = Vec::with_capacity(text.len() * 2);
    for character in text.chars() {
        let code = font.character_codes.get(&character).ok_or_else(|| {
            ScanError::Other("searchable PDF text is missing a Type0 character mapping".into())
        })?;
        encoded.extend_from_slice(&code.to_be_bytes());
    }
    Ok(encoded)
}

fn searchable_to_unicode_cmap(character_codes: &BTreeMap<char, u16>) -> Vec<u8> {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo\n<< /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /OpenScanline-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let mappings = character_codes
        .iter()
        .map(|(character, code)| (*code, *character))
        .collect::<Vec<_>>();
    for chunk in mappings.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (code, character) in chunk {
            let utf16 = character
                .encode_utf16(&mut [0_u16; 2])
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect::<String>();
            let _ = writeln!(cmap, "<{code:04X}> <{utf16}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap.into_bytes()
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

fn configure_pdf_document(
    document: &mut Document,
    pages_id: ObjectId,
    page_ids: &[ObjectId],
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    check_pdf_cancellation(cancellation)?;
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_ids.len() as i64,
        }),
    );
    check_pdf_cancellation(cancellation)?;
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    check_pdf_cancellation(cancellation)?;
    let info_id = document.add_object(dictionary! {
        "Title" => unicode_pdf_string(&options.title),
        "Producer" => Object::string_literal("open-scanline"),
    });
    document.trailer.set("Root", catalog_id);
    document.trailer.set("Info", info_id);
    let id_bytes = random_bytes::<16>("PDF document identifier")?.to_vec();
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal(id_bytes.clone()),
            Object::string_literal(id_bytes),
        ],
    );
    Ok(())
}

fn encrypt_pdf_document(document: &mut Document, options: &PdfOptions) -> Result<()> {
    let Some(password) = options.password.as_deref() else {
        return Ok(());
    };
    let filter_name = b"StdCF".to_vec();
    let crypt_filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let file_encryption_key = random_bytes::<32>("PDF AES-256 encryption key")?;
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(filter_name.clone(), crypt_filter)]),
        file_encryption_key: &file_encryption_key,
        stream_filter: filter_name.clone(),
        string_filter: filter_name,
        owner_password: password,
        user_password: password,
        permissions: Permissions::all(),
    })
    .map_err(|error| ScanError::Other(format!("PDF encryption setup failed: {error}")))?;
    document
        .encrypt(&state)
        .map_err(|error| ScanError::Other(format!("PDF encryption failed: {error}")))?;
    let encryption_id = document
        .trailer
        .get(b"Encrypt")
        .and_then(Object::as_reference)
        .map_err(|error| ScanError::Other(format!("PDF encryption dictionary missing: {error}")))?;
    document
        .get_dictionary_mut(encryption_id)
        .map_err(|error| ScanError::Other(format!("PDF encryption dictionary invalid: {error}")))?
        .set("Length", 256);
    Ok(())
}

/// Encode a PDF text string as UTF-16BE with the required byte-order marker.
fn unicode_pdf_string(text: &str) -> Object {
    let mut encoded = Vec::with_capacity(2 + text.len() * 2);
    encoded.extend_from_slice(&[0xFE, 0xFF]);
    encoded.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    Object::String(encoded, StringFormat::Hexadecimal)
}

pub(crate) fn validate_pdf_password(password: &str) -> Result<()> {
    let prepared = stringprep::saslprep(password).map_err(|error| {
        ScanError::Invalid(format!(
            "PDF password is not valid SASLprep Unicode: {error}"
        ))
    })?;
    if prepared.is_empty() {
        return Err(ScanError::Invalid(
            "PDF password must not be empty after SASLprep normalization".into(),
        ));
    }
    if prepared.len() > 127 {
        return Err(ScanError::Invalid(
            "PDF passwords are limited to 127 UTF-8 bytes after SASLprep normalization".into(),
        ));
    }
    Ok(())
}

fn random_bytes<const N: usize>(purpose: &str) -> Result<[u8; N]> {
    let mut bytes = [0_u8; N];
    getrandom::fill(&mut bytes).map_err(|error| {
        ScanError::Other(format!("could not generate secure {purpose}: {error}"))
    })?;
    Ok(bytes)
}

fn save_pdf_document(
    document: &mut Document,
    path: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    document.compress();
    let temp = create_output_temp(path)?;
    document
        .save(temp.path())
        .map_err(|error| ScanError::Image(format!("PDF save failed: {error}")))?;
    validate_output_container(temp.path(), "pdf")?;
    check_pdf_cancellation(cancellation)?;
    temp.publish()?;
    Ok(())
}

/// Combine on-disk page images into one multipage PDF.
pub fn save_multipage_pdf(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options(pages, out, options)
}

/// Combine page images while allowing cancellation before each page and publication.
pub fn save_multipage_pdf_with_cancellation(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_cancellation(pages, out, options, cancellation)
}

/// Build a PDF while loading only one raw page image at a time. The PDF
/// document retains its compressed image streams, but large decoded page
/// buffers are released before the next page is read.
pub fn save_pdf_from_paths_with_options(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_cancellation(pages, out, options, None)
}

/// Build a PDF from page paths while allowing cancellation before each page
/// and immediately before atomic publication.
pub fn save_pdf_from_paths_with_options_and_cancellation(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_transform_and_cancellation(
        pages,
        out,
        options,
        |image| Ok(image.clone()),
        cancellation,
    )
}

/// Build a PDF from paths while transforming one decoded page at a time.
///
/// This keeps the decoded source-page lifetime bounded to the current page;
/// the PDF document still owns its encoded page streams until publication.
pub fn save_pdf_from_paths_with_options_and_transform<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    transform: F,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    save_pdf_from_paths_with_options_and_transform_and_cancellation(
        pages, out, options, transform, None,
    )
}

/// Build a transformed PDF while allowing cancellation before every page,
/// transform, object append, and atomic publication.
pub fn save_pdf_from_paths_with_options_and_transform_and_cancellation<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    mut transform: F,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    if pages.is_empty() {
        return Err(ScanError::Invalid(
            "save_multipage_pdf requires at least one page".into(),
        ));
    }
    validate_pdf_options(pages.len(), options)?;
    check_pdf_cancellation(cancellation)?;
    let path = out.as_ref();
    create_pdf_parent(path)?;
    let (mut document, pages_id, searchable_font) = create_pdf_document(options, cancellation)?;
    let mut page_ids = Vec::with_capacity(pages.len());
    {
        let mut builder = PdfPageBuilder {
            document: &mut document,
            context: PdfPageContext {
                pages_id,
                searchable_font,
                dpi: options.dpi.max(1) as f64,
            },
            retained_image_stream_bytes: 0,
        };
        for (index, page) in pages.iter().enumerate() {
            check_pdf_cancellation(cancellation)?;
            let image = load_image(page)?;
            check_pdf_cancellation(cancellation)?;
            let image = transform(&image)?;
            check_pdf_cancellation(cancellation)?;
            let searchable_text = options
                .searchable_pages
                .as_ref()
                .map(|text| text[index].as_str());
            page_ids.push(builder.add_page(index, &image, searchable_text, cancellation)?);
        }
    }
    finish_pdf_document(
        &mut document,
        pages_id,
        &page_ids,
        path,
        options,
        cancellation,
    )?;
    Ok(path.to_path_buf())
}
