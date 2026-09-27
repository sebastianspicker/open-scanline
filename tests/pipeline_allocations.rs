//! Resource regression: disabled pipeline stages must not clone their inputs.
use open_scanline::core::{ImageBuffer, PipelinePrefs, PixelFormat};
use open_scanline::pipeline::apply_pipeline;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATED: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn record(bytes: usize) {
    let _ = ALLOCATED.try_with(|counter| {
        if let Some((count, total)) = counter.get() {
            counter.set(Some((count + 1, total + bytes)));
        }
    });
}

// SAFETY: the allocator delegates every operation to System without changing pointers.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        record(bytes);
        unsafe { System.realloc(pointer, layout, bytes) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[test]
fn borrowed_default_pipeline_allocates_exactly_one_image() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image =
            ImageBuffer::new(1000, 1000, format, vec![17; 1_000_000 * format.bpp()]).unwrap();
        let prefs = PipelinePrefs::default();
        ALLOCATED.with(|counter| counter.set(Some((0, 0))));
        let result = apply_pipeline(&image, &prefs).unwrap();
        let measured = ALLOCATED.with(|counter| counter.replace(None)).unwrap();
        assert_eq!(measured, (1, image.data.len()));
        assert_eq!(result, image);
    }
}
