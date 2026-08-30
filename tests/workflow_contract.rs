use open_scanline::batch::{run_batch_scan, BatchScanArgs};
use open_scanline::core::{ImageBuffer, PipelinePrefs, ScanError};
use open_scanline::device::CancellationToken;
use open_scanline::imaging::{load_image, save_image};
use open_scanline::process::{process_image_file, ProcessOptions};
use open_scanline::scan::{run_scan_to_file, run_scan_to_file_with_token, ScanToFileArgs};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn mock_scan_publishes_final_and_unprocessed_raw_images() {
    let directory = ScratchDirectory::new("scan-contract");
    let final_path = directory.path().join("final.png");
    let raw_path = directory.path().join("raw.tif");
    let result = run_scan_to_file(ScanToFileArgs {
        out: final_path.clone(),
        raw_out: Some(raw_path.clone()),
        width: 2,
        height: 2,
        seed: 17,
        invert_colors: true,
        ..ScanToFileArgs::default()
    })
    .expect("mock scan should complete");

    assert_eq!(result, final_path);
    let raw = load_image(&raw_path).expect("raw image should be readable");
    assert_eq!(raw.width, 2);
    assert_eq!(raw.height, 2);
    assert_eq!(
        raw.data,
        vec![0, 0, 17, 255, 0, 17, 0, 255, 17, 255, 255, 17]
    );

    let final_image = load_image(&final_path).expect("final image should be readable");
    assert_eq!(
        final_image.data,
        vec![255, 255, 238, 0, 255, 238, 255, 0, 238, 0, 0, 238]
    );
}

#[test]
fn cancelled_scan_does_not_publish_any_destination() {
    let directory = ScratchDirectory::new("cancel-contract");
    let final_path = directory.path().join("final.png");
    let raw_path = directory.path().join("raw.tif");
    let token = CancellationToken::new();
    token.cancel();

    let result = run_scan_to_file_with_token(
        ScanToFileArgs {
            out: final_path.clone(),
            raw_out: Some(raw_path.clone()),
            ..ScanToFileArgs::default()
        },
        token,
    );

    assert!(matches!(result, Err(ScanError::Cancelled(_))));
    assert!(
        !final_path.exists(),
        "cancelled scan published final output"
    );
    assert!(!raw_path.exists(), "cancelled scan published raw output");
}

#[test]
fn mock_batch_publishes_pages_in_seed_order() {
    let directory = ScratchDirectory::new("batch-contract");
    let out_dir = directory.path().join("pages");
    let paths = run_batch_scan(BatchScanArgs {
        out_dir: out_dir.clone(),
        pages: 3,
        width: 2,
        height: 1,
        seed: 41,
        ..BatchScanArgs::default()
    })
    .expect("mock batch should complete");

    assert_eq!(
        paths,
        vec![
            out_dir.join("page_001.png"),
            out_dir.join("page_002.png"),
            out_dir.join("page_003.png"),
        ]
    );
    for (index, path) in paths.iter().enumerate() {
        let page = load_image(path).expect("batch page should be readable");
        let seed = 41 + index as u8;
        assert_eq!(page.data, vec![0, 0, seed, 255, 0, seed]);
    }
}

#[test]
fn batch_rejects_aliased_and_non_file_destinations_before_publication() {
    let directory = ScratchDirectory::new("batch-destination-contract");
    let aliased_out_dir = directory.path().join("aliased-pages");
    let alias_result = run_batch_scan(BatchScanArgs {
        out_dir: aliased_out_dir.clone(),
        pages: 1,
        contact_sheet: Some(aliased_out_dir.join("page_001.png")),
        ..BatchScanArgs::default()
    });
    assert!(matches!(alias_result, Err(ScanError::Invalid(_))));
    assert!(
        !aliased_out_dir.exists(),
        "destination validation created page outputs"
    );

    let unsafe_out_dir = directory.path().join("unsafe-pages");
    let directory_destination = directory.path().join("not-a-file.pdf");
    fs::create_dir(&directory_destination).expect("fixture directory should be created");
    let unsafe_result = run_batch_scan(BatchScanArgs {
        out_dir: unsafe_out_dir.clone(),
        pages: 1,
        multipage_pdf: Some(directory_destination),
        ..BatchScanArgs::default()
    });
    assert!(matches!(unsafe_result, Err(ScanError::Invalid(_))));
    assert!(
        !unsafe_out_dir.exists(),
        "unsafe destination validation created page outputs"
    );
}

#[test]
fn process_validation_failure_preserves_an_existing_destination() {
    let directory = ScratchDirectory::new("process-contract");
    let source = directory.path().join("source.png");
    let destination = directory.path().join("preserved.unsupported");
    save_image(
        &source,
        &ImageBuffer::new(1, 1, open_scanline::core::PixelFormat::Rgb8, vec![1, 2, 3]).unwrap(),
        None,
        None,
    )
    .expect("source fixture should be written");
    fs::write(&destination, b"must survive validation failure").unwrap();

    let result = process_image_file(&ProcessOptions {
        src: source,
        dst: destination.clone(),
        pipeline: PipelinePrefs::default(),
        quality: None,
    });

    assert!(matches!(result, Err(ScanError::Invalid(_))));
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"must survive validation failure"
    );
}

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("scratch directory should be unique");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
