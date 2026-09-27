use super::*;
use crate::domain::image::{ImageBuffer, PixelFormat};
use std::cell::Cell;

fn decode_fixture(calls: &Cell<usize>) -> crate::error::Result<ImageBuffer> {
    calls.set(calls.get() + 1);
    ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![0, 0, 0, 255, 255, 255])
}

#[test]
fn redraw_reuses_one_decode_and_invalidation_rebuilds_both_caches() {
    let mut app = OpenScanlineApp::new(None);
    let ctx = egui::Context::default();
    let calls = Cell::new(0);
    app.state.last_image = Some("fixture.png".into());
    for _ in 0..25 {
        app.ensure_preview_with(&ctx, |_| decode_fixture(&calls));
    }
    assert_eq!(calls.get(), 1);
    assert!(app.preview_tex.is_some());
    let statistics = app.state.preview.as_ref().unwrap();
    assert_eq!((statistics.width, statistics.height), (2, 1));
    assert_eq!(statistics.luma.iter().sum::<u64>(), 2);
    app.state.update_histogram();
    app.ensure_preview_with(&ctx, |_| decode_fixture(&calls));
    assert_eq!(calls.get(), 2);
    app.state.last_image = None;
    app.ensure_preview_with(&ctx, |_| panic!("empty preview must not decode"));
    assert!(app.preview_tex.is_none());
    assert!(app.state.preview.is_none());
    assert!(app.state.hist_summary.is_empty());
}

#[test]
fn failed_replacement_clears_all_statistics_and_is_not_retried_on_redraw() {
    let mut app = OpenScanlineApp::new(None);
    let ctx = egui::Context::default();
    app.state.last_image = Some("first.png".into());
    app.ensure_preview_with(&ctx, |_| decode_fixture(&Cell::new(0)));
    app.state.last_image = Some("broken.png".into());
    app.ensure_preview_with(&ctx, |_| Err(ScanError::Other("decode fixture".into())));
    assert!(app.preview_tex.is_none());
    assert!(app.state.preview.is_none());
    assert!(app.state.hist_summary.is_empty());
    app.ensure_preview_with(&ctx, |_| panic!("failed fingerprint must be cached"));
    app.state.update_histogram();
    app.ensure_preview_with(&ctx, |_| decode_fixture(&Cell::new(0)));
    assert!(app.preview_tex.is_some());
    assert!(app.state.preview.is_some());
}

#[test]
fn subsecond_same_size_edits_invalidate_preview() {
    let output = TemporaryOutput::new("preview-fingerprint", "png").unwrap();
    let file = std::fs::File::create(output.path()).unwrap();
    let first = std::time::UNIX_EPOCH + std::time::Duration::new(100, 100_000_000);
    file.set_times(std::fs::FileTimes::new().set_modified(first))
        .unwrap();
    let before = preview_fingerprint(output.path());
    let second = first + std::time::Duration::from_millis(100);
    file.set_times(std::fs::FileTimes::new().set_modified(second))
        .unwrap();
    assert_ne!(before, preview_fingerprint(output.path()));
}
