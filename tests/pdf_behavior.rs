use lopdf::Document;
use open_scanline::core::{ImageBuffer, PixelFormat};
use open_scanline::imaging::{save_pdf_with_options, PdfOptions};
use std::process::Command;

fn sample(width: u32, height: u32, value: u8) -> ImageBuffer {
    ImageBuffer::new(
        width,
        height,
        PixelFormat::Rgb8,
        vec![value; (width * height * 3) as usize],
    )
    .unwrap()
}

#[test]
fn structured_pdf_has_pages_title_searchable_text_and_dimensions() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("searchable.pdf");
    save_pdf_with_options(
        &path,
        &[sample(300, 150, 20), sample(150, 300, 220)],
        &PdfOptions {
            dpi: 150,
            title: "Behavior proof".into(),
            password: None,
            searchable_pages: Some(vec!["FIRST PAGE".into(), "SECOND PAGE".into()]),
        },
    )
    .unwrap();
    let document = Document::load(&path).unwrap();
    assert_eq!(document.get_pages().len(), 2);
    let text = document.extract_text(&[1, 2]).unwrap();
    assert!(text.contains("FIRST PAGE"));
    assert!(text.contains("SECOND PAGE"));
    let info = document
        .trailer
        .get(b"Info")
        .unwrap()
        .as_reference()
        .unwrap();
    let title = document
        .get_dictionary(info)
        .unwrap()
        .get(b"Title")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(decode_utf16be_pdf_string(title), "Behavior proof");
    let first_page = *document.get_pages().get(&1).unwrap();
    let media_box = document
        .get_dictionary(first_page)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(media_box[2].as_float().unwrap(), 144.0);
    assert_eq!(media_box[3].as_float().unwrap(), 72.0);
}

#[test]
fn encrypted_pdf_requires_and_accepts_the_password() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("encrypted.pdf");
    save_pdf_with_options(
        &path,
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some("secret".into()),
            ..PdfOptions::default()
        },
    )
    .unwrap();
    assert!(Document::load_with_password(&path, "wrong").is_err());
    let document = Document::load_with_password(&path, "secret").unwrap();
    assert_eq!(document.get_pages().len(), 1);
}

#[test]
fn encrypted_pdfs_use_aes_and_unique_key_material() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("encrypted-first.pdf");
    let second = dir.join("encrypted-second.pdf");
    let options = PdfOptions {
        password: Some("secret".into()),
        ..PdfOptions::default()
    };

    save_pdf_with_options(&first, &[sample(32, 24, 80)], &options).unwrap();
    save_pdf_with_options(&second, &[sample(32, 24, 80)], &options).unwrap();

    assert_ne!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap()
    );
    assert_aes_encryption(&first);
    assert_aes_encryption(&second);
    assert!(Document::load_with_password(&first, "wrong").is_err());
    assert_eq!(
        Document::load_with_password(&first, "secret")
            .unwrap()
            .get_pages()
            .len(),
        1
    );
}

#[test]
fn aes256_passwords_do_not_collide_after_the_legacy_32_byte_boundary() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("aes256-password-boundary.pdf");
    let password = format!("{}A", "a".repeat(32));
    let different_suffix = format!("{}B", "a".repeat(32));

    save_pdf_with_options(
        &path,
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some(password.clone()),
            ..PdfOptions::default()
        },
    )
    .unwrap();

    assert!(Document::load_with_password(&path, &different_suffix).is_err());
    assert_eq!(
        Document::load_with_password(&path, &password)
            .unwrap()
            .get_pages()
            .len(),
        1
    );
}

#[test]
fn aes256_accepts_unicode_passwords_and_rejects_pdf_limit_overflow() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("aes256-unicode-password.pdf");
    let password = "пароль 世界";

    save_pdf_with_options(
        &path,
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some(password.into()),
            ..PdfOptions::default()
        },
    )
    .unwrap();

    assert!(Document::load_with_password(&path, "пароль 世").is_err());
    assert_eq!(
        Document::load_with_password(&path, password)
            .unwrap()
            .get_pages()
            .len(),
        1
    );
    let error = save_pdf_with_options(
        dir.join("aes256-overflow-password.pdf"),
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some("a".repeat(128)),
            ..PdfOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("127 UTF-8 bytes"));
    let error = save_pdf_with_options(
        dir.join("aes256-empty-password.pdf"),
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some(String::new()),
            ..PdfOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("must not be empty"));
    let error = save_pdf_with_options(
        dir.join("aes256-saslprep-empty-password.pdf"),
        &[sample(32, 24, 80)],
        &PdfOptions {
            password: Some("\u{00ad}".into()),
            ..PdfOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("after SASLprep"));
}

#[test]
fn searchable_pdf_extracts_unicode_text_and_title_in_plain_and_encrypted_documents() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let plain = dir.join("searchable-unicode.pdf");
    let encrypted = dir.join("searchable-unicode-encrypted.pdf");
    let text = "Привет 世界";
    let title = "Отчёт 世界";
    let options = PdfOptions {
        title: title.into(),
        searchable_pages: Some(vec![text.into()]),
        ..PdfOptions::default()
    };

    save_pdf_with_options(&plain, &[sample(300, 150, 80)], &options).unwrap();
    assert_unicode_pdf_contents(&plain, text, title, None);

    let password = "external-proof-password";
    save_pdf_with_options(
        &encrypted,
        &[sample(300, 150, 80)],
        &PdfOptions {
            password: Some(password.into()),
            ..options
        },
    )
    .unwrap();
    assert_unicode_pdf_contents(&encrypted, text, title, Some(password));
}

#[test]
fn pdf_encode_failure_preserves_existing_output() {
    let dir = std::env::temp_dir().join("open_scanline_pdf_behavior");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preserve-on-failure.pdf");
    std::fs::write(&path, b"previous PDF output").unwrap();
    let malformed = ImageBuffer {
        width: 2,
        height: 2,
        pixel_format: PixelFormat::Rgb8,
        data: vec![0; 3],
    };

    let error = save_pdf_with_options(&path, &[malformed], &PdfOptions::default()).unwrap_err();

    assert!(error.to_string().contains("rgb buffer"));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous PDF output");
}

#[cfg(not(windows))]
#[test]
fn pdf_publication_replaces_the_destination_only_with_a_valid_document() {
    let dir = std::env::temp_dir().join(format!("open_scanline_pdf_atomic_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("atomic-replacement.pdf");
    std::fs::write(&path, b"previous PDF output").unwrap();

    save_pdf_with_options(&path, &[sample(32, 24, 80)], &PdfOptions::default()).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(Document::load(&path).is_ok());
    assert_eq!(
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("open-scanline"))
            .count(),
        0,
        "successful publication must not leave sibling temporary files"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_aes_encryption(path: &std::path::Path) {
    let document = Document::load(path).unwrap();
    assert_eq!(document.version, "2.0");
    let encryption_id = document
        .trailer
        .get(b"Encrypt")
        .unwrap()
        .as_reference()
        .unwrap();
    let encryption = document.get_dictionary(encryption_id).unwrap();
    assert_eq!(encryption.get(b"V").unwrap().as_i64().unwrap(), 5);
    assert_eq!(encryption.get(b"R").unwrap().as_i64().unwrap(), 6);
    assert_eq!(encryption.get(b"Length").unwrap().as_i64().unwrap(), 256);
    let standard_filter = encryption
        .get(b"CF")
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"StdCF")
        .unwrap()
        .as_dict()
        .unwrap();
    assert_eq!(
        standard_filter.get(b"CFM").unwrap().as_name().unwrap(),
        b"AESV3"
    );
}

fn assert_unicode_pdf_contents(
    path: &std::path::Path,
    expected_text: &str,
    expected_title: &str,
    password: Option<&str>,
) {
    let document = match password {
        Some(password) => Document::load_with_password(path, password).unwrap(),
        None => Document::load(path).unwrap(),
    };
    assert!(document.extract_text(&[1]).unwrap().contains(expected_text));
    let info = document
        .trailer
        .get(b"Info")
        .unwrap()
        .as_reference()
        .unwrap();
    let title = document
        .get_dictionary(info)
        .unwrap()
        .get(b"Title")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(decode_utf16be_pdf_string(title), expected_title);
    assert_pdftotext_extracts(path, expected_text, password);
}

fn decode_utf16be_pdf_string(bytes: &[u8]) -> String {
    assert!(bytes.starts_with(&[0xFE, 0xFF]));
    String::from_utf16(
        &bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn assert_pdftotext_extracts(path: &std::path::Path, expected_text: &str, password: Option<&str>) {
    let mut command = Command::new("pdftotext");
    if let Some(password) = password {
        command.args(["-upw", password]);
    }
    match command.arg(path).arg("-").output() {
        Ok(output) => {
            assert!(
                output.status.success(),
                "pdftotext rejected generated PDF: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                collapse_whitespace(&String::from_utf8_lossy(&output.stdout))
                    .contains(&collapse_whitespace(expected_text)),
                "pdftotext did not extract the expected Unicode text from {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stdout)
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("could not run pdftotext: {error}"),
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
