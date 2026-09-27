pub mod media;
use open_scanline::core::ImageBuffer;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
static COUNT: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
pub struct CountingAllocator;
// SAFETY: all allocation operations delegate to System with unchanged arguments.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        COUNT.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size as u64, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, size) }
    }
}
pub fn selected(operation: &str) -> bool {
    std::env::var("OPEN_SCANLINE_BENCH_OPERATIONS")
        .map(|names| names.split(',').any(|name| name == operation))
        .unwrap_or(true)
}

pub fn measure(name: &str, image: &ImageBuffer, mut run: impl FnMut()) -> Value {
    eprintln!(
        "{name}: {}x{} {}",
        image.width,
        image.height,
        image.pixel_format.as_str()
    );
    for _ in 0..3 {
        run();
    }
    let mut samples = Vec::with_capacity(10);
    for _ in 0..10 {
        let count = COUNT.load(Ordering::Relaxed);
        let bytes = BYTES.load(Ordering::Relaxed);
        let start = Instant::now();
        run();
        let elapsed = start.elapsed().as_nanos() as u64;
        let allocations = COUNT.load(Ordering::Relaxed) - count;
        let allocated = BYTES.load(Ordering::Relaxed) - bytes;
        samples
            .push(json!({"ns": elapsed, "allocations": allocations, "allocated_bytes": allocated}));
    }
    json!({"operation": name, "width": image.width, "height": image.height,
        "format": image.pixel_format.as_str(), "samples": samples})
}
pub fn peak_rss() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes the writable rusage on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let value = unsafe { usage.assume_init() }.ru_maxrss as u64;
            return Some(if cfg!(target_os = "macos") {
                value
            } else {
                value * 1024
            });
        }
    }
    None
}
