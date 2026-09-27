use super::page::{PdfPageBuilder, PdfPageContext};
use super::searchable::{add_searchable_font, validate_pdf_searchable_pages, SearchableFont};
use super::{check_pdf_cancellation, validate_pdf_password};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::codecs::{create_output_temp, validate_output_container};
use crate::operation::CancellationToken;
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{
    dictionary, Document, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions,
    StringFormat,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Options for structured PDF output.
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
        append_buffer_pages(&mut pages, images, options, cancellation, &mut page_ids)?;
    }
    finish_pdf_document(&mut doc, pages_id, &page_ids, path, options, cancellation)?;
    Ok(path.to_path_buf())
}

fn append_buffer_pages(
    pages: &mut PdfPageBuilder<'_>,
    images: &[ImageBuffer],
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
    page_ids: &mut Vec<ObjectId>,
) -> Result<()> {
    for (index, image) in images.iter().enumerate() {
        check_pdf_cancellation(cancellation)?;
        let text = options
            .searchable_pages
            .as_ref()
            .map(|pages| pages[index].as_str());
        page_ids.push(pages.add_page(index, image, text, cancellation)?);
    }
    Ok(())
}

pub(super) fn create_pdf_document(
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

pub(super) fn finish_pdf_document(
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

pub(super) fn validate_pdf_options(page_count: usize, options: &PdfOptions) -> Result<()> {
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

pub(super) fn create_pdf_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
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
