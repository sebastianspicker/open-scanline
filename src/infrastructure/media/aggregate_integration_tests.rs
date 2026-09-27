use super::{save_image, NativeAggregateSession};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::workflows::capture::batch::{outputs::write_requested_outputs, BatchScanArgs};
use crate::workflows::ports::media::{AggregateMediaSession, PdfPathPublication};
use crate::workflows::publication::{
    prepare_export_options_for_pdf_with_media, ExportOptions, PreparedExportOptions,
};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-aggregate-integration-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn jpeg_pages(scratch: &Scratch) -> Vec<PathBuf> {
    (0..2)
        .map(|index| {
            let path = scratch.path(&format!("page-{index}.jpg"));
            let image =
                ImageBuffer::new(8, 8, PixelFormat::Rgb8, vec![40 + index as u8; 8 * 8 * 3])
                    .unwrap();
            save_image(&path, &image, Some(150), Some(70)).unwrap();
            path
        })
        .collect()
}

fn batch_args(root: &Path) -> BatchScanArgs {
    BatchScanArgs {
        out_dir: root.join("pages"),
        multipage_out: Some(root.join("alias.pdf")),
        multipage_tiff: Some(root.join("named.tiff")),
        multipage_pdf: Some(root.join("named.pdf")),
        contact_sheet: Some(root.join("sheet.png")),
        ..BatchScanArgs::default()
    }
}

fn prepared() -> PreparedExportOptions {
    prepare_export_options_for_pdf_with_media(true, &ExportOptions::default(), &super::NativeMedia)
        .unwrap()
}

fn assert_later_outputs_absent(args: &BatchScanArgs) {
    assert!(!args.multipage_tiff.as_ref().unwrap().exists());
    assert!(!args.multipage_pdf.as_ref().unwrap().exists());
    assert!(!args.contact_sheet.as_ref().unwrap().exists());
}

fn publish_pdf(session: &mut NativeAggregateSession, pages: &[PathBuf], destination: &Path) {
    session
        .publish_pdf(PdfPathPublication {
            paths: pages,
            destination,
            dpi: 150,
            title: "cache integration",
            password: None,
            searchable_pages: None,
            transform: None,
            cancellation: None,
        })
        .unwrap();
}

#[test]
fn all_aggregate_writers_share_one_decode_per_published_jpeg_page() {
    let scratch = Scratch::new();
    let pages = jpeg_pages(&scratch);
    let mut session = NativeAggregateSession {
        loader: super::aggregate_cache::SharedPageLoader::new(),
    };
    publish_pdf(&mut session, &pages, &scratch.path("alias.pdf"));
    session
        .publish_tiff(&pages, &scratch.path("named.tiff"), Some(150), None, None)
        .unwrap();
    publish_pdf(&mut session, &pages, &scratch.path("named.pdf"));
    session
        .publish_contact_sheet(&pages, &scratch.path("sheet.png"), None)
        .unwrap();
    assert_eq!(session.loader.decode_count(), pages.len());
    for name in ["alias.pdf", "named.tiff", "named.pdf", "sheet.png"] {
        assert!(scratch.path(name).is_file());
    }
}

#[test]
fn source_failure_in_first_aggregate_stops_later_outputs() {
    let scratch = Scratch::new();
    let args = batch_args(&scratch.0);
    let missing = vec![scratch.path("missing.png")];
    assert!(write_requested_outputs(
        &args,
        &missing,
        &prepared(),
        Vec::new(),
        None,
        None,
        &super::NativeMedia,
    )
    .is_err());
    assert_later_outputs_absent(&args);
}

#[test]
fn first_encoder_setup_failure_stops_later_outputs() {
    let scratch = Scratch::new();
    let mut args = batch_args(&scratch.0);
    let blocker = scratch.path("blocker");
    std::fs::write(&blocker, b"file blocks output parent").unwrap();
    args.multipage_out = Some(blocker.join("alias.pdf"));
    let source = scratch.path("page.png");
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![1, 2, 3]).unwrap();
    save_image(&source, &image, None, None).unwrap();
    assert!(write_requested_outputs(
        &args,
        &[source],
        &prepared(),
        Vec::new(),
        None,
        None,
        &super::NativeMedia,
    )
    .is_err());
    assert_later_outputs_absent(&args);
}

#[test]
fn later_pdf_failure_retains_alias_and_tiff_publications() {
    let scratch = Scratch::new();
    let pages = jpeg_pages(&scratch);
    let mut args = batch_args(&scratch.0);
    let blocker = scratch.path("blocker");
    std::fs::write(&blocker, b"file blocks named PDF parent").unwrap();
    args.multipage_pdf = Some(blocker.join("named.pdf"));
    assert!(write_requested_outputs(
        &args,
        &pages,
        &prepared(),
        Vec::new(),
        None,
        None,
        &super::NativeMedia,
    )
    .is_err());
    assert!(args.multipage_out.as_ref().unwrap().is_file());
    assert!(args.multipage_tiff.as_ref().unwrap().is_file());
    assert!(!args.multipage_pdf.as_ref().unwrap().exists());
    assert!(!args.contact_sheet.as_ref().unwrap().exists());
}

#[test]
fn cancellation_between_outputs_retains_completed_publications() {
    let scratch = Scratch::new();
    let pages = jpeg_pages(&scratch);
    let args = batch_args(&scratch.0);
    let tiff = args.multipage_tiff.as_ref().unwrap().clone();
    let cancel = move || tiff.is_file();
    let result = write_requested_outputs(
        &args,
        &pages,
        &prepared(),
        Vec::new(),
        Some(&cancel),
        None,
        &super::NativeMedia,
    );
    assert!(matches!(result, Err(crate::error::ScanError::Cancelled(_))));
    assert!(args.multipage_out.as_ref().unwrap().is_file());
    assert!(args.multipage_tiff.as_ref().unwrap().is_file());
    assert!(!args.multipage_pdf.as_ref().unwrap().exists());
    assert!(!args.contact_sheet.as_ref().unwrap().exists());
}
