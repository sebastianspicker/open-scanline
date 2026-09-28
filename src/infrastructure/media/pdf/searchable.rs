use super::check_pdf_cancellation;
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const SEARCHABLE_FONT_BYTES: &[u8] =
    include_bytes!("../../../../assets/fonts/Cantarell-Regular.ttf");

/// PDF construction retains each compressed page stream until the document is
/// atomically published. Limit the aggregate compressed streams to half the
/// product's 512 MiB decoded-image ceiling, leaving room for the current page
/// and PDF object/compression overhead.
/// OCR text is copied into the font map, CMap, and page content stream. A
/// 1 MiB page limit keeps one pathological OCR result from dominating those
/// allocations.
pub(crate) const MAX_PDF_SEARCHABLE_PAGE_UTF8_BYTES: usize = 1024 * 1024;
/// The aggregate searchable layer stays well below the retained 256 MiB image
/// stream budget, including for long feeder batches.
pub(crate) const MAX_PDF_SEARCHABLE_TEXT_UTF8_BYTES: usize = 16 * 1024 * 1024;
pub(super) struct SearchableFont {
    pub(super) object_id: ObjectId,
    pub(super) character_codes: BTreeMap<char, u16>,
}

pub(super) fn validate_pdf_searchable_pages(searchable_pages: &[String]) -> Result<()> {
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

pub(super) fn add_searchable_font(
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
    let references = create_font_references(document, &character_codes, cancellation)?;
    let object_id = create_type0_font(document, references, cancellation)?;
    Ok(Some(SearchableFont {
        object_id,
        character_codes,
    }))
}

struct SearchableFontReferences {
    to_unicode_id: ObjectId,
    descriptor_id: ObjectId,
    cid_to_gid_id: ObjectId,
}

fn create_font_references(
    document: &mut Document,
    character_codes: &BTreeMap<char, u16>,
    cancellation: Option<&CancellationToken>,
) -> Result<SearchableFontReferences> {
    let to_unicode_id = add_font_object(
        document,
        dictionary! {},
        searchable_to_unicode_cmap(character_codes),
        cancellation,
    )?;
    let font_file_id = add_font_object(
        document,
        dictionary! {
            "Length1" => SEARCHABLE_FONT_BYTES.len() as i64,
            "Filter" => "Crypt",
            "DecodeParms" => dictionary! { "Name" => "Identity" },
        },
        SEARCHABLE_FONT_BYTES.to_vec(),
        cancellation,
    )?;
    let descriptor_id = add_font_dictionary(
        document,
        dictionary! {
            "Type" => "FontDescriptor", "FontName" => "Cantarell-Regular", "Flags" => 4,
            "FontBBox" => vec![0.into(), 0.into(), 1000.into(), 1000.into()], "ItalicAngle" => 0,
            "Ascent" => 880, "Descent" => -120, "CapHeight" => 700, "StemV" => 80,
            "FontFile2" => font_file_id,
        },
        cancellation,
    )?;
    let cid_to_gid_id = add_font_object(
        document,
        dictionary! {},
        cid_to_gid_map(character_codes.len()),
        cancellation,
    )?;
    Ok(SearchableFontReferences {
        to_unicode_id,
        descriptor_id,
        cid_to_gid_id,
    })
}

fn create_type0_font(
    document: &mut Document,
    references: SearchableFontReferences,
    cancellation: Option<&CancellationToken>,
) -> Result<ObjectId> {
    let cid_font_id = add_font_dictionary(
        document,
        dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Cantarell-Regular",
            "CIDSystemInfo" => dictionary! {
                "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0,
            },
            "FontDescriptor" => references.descriptor_id, "DW" => 1000,
            "CIDToGIDMap" => references.cid_to_gid_id,
        },
        cancellation,
    )?;
    add_font_dictionary(
        document,
        dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Cantarell-Regular",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![cid_font_id.into()],
            "ToUnicode" => references.to_unicode_id,
        },
        cancellation,
    )
}

fn add_font_object(
    document: &mut Document,
    dictionary: lopdf::Dictionary,
    content: Vec<u8>,
    cancellation: Option<&CancellationToken>,
) -> Result<ObjectId> {
    check_pdf_cancellation(cancellation)?;
    Ok(document.add_object(Stream::new(dictionary, content)))
}

fn add_font_dictionary(
    document: &mut Document,
    dictionary: lopdf::Dictionary,
    cancellation: Option<&CancellationToken>,
) -> Result<ObjectId> {
    check_pdf_cancellation(cancellation)?;
    Ok(document.add_object(dictionary))
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

pub(super) fn encode_searchable_text(text: &str, font: &SearchableFont) -> Result<Vec<u8>> {
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
