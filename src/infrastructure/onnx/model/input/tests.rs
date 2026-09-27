use super::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

mod benchmark;

thread_local! {
    static ALLOCATED: Cell<Option<AllocationSample>> = const { Cell::new(None) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AllocationSample {
    pub(super) allocations: usize,
    pub(super) bytes: usize,
}

fn record_allocation(bytes: usize) {
    let _ = ALLOCATED.try_with(|counter| {
        if let Some(mut sample) = counter.get() {
            sample.allocations += 1;
            sample.bytes += bytes;
            counter.set(Some(sample));
        }
    });
}

// SAFETY: every operation delegates to System with its pointer and layout unchanged.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation(size);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

pub(super) fn measure_allocations<T>(run: impl FnOnce() -> T) -> (T, AllocationSample) {
    ALLOCATED.with(|counter| counter.set(Some(AllocationSample::default())));
    let output = run();
    let sample = ALLOCATED
        .with(|counter| counter.replace(None))
        .expect("allocation measurement was enabled");
    (output, sample)
}

pub(super) fn fixture(width: u32, height: u32, format: PixelFormat, seed: u64) -> ImageBuffer {
    let mut state = seed;
    let length = width as usize * height as usize * format.bpp();
    let data = (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    ImageBuffer::new(width, height, format, data).unwrap()
}

pub(super) fn oracle_image_tensor(
    image: &ImageBuffer,
    shape: &[usize],
    layout: OnnxInputLayout,
    normalization: OnnxNormalization,
) -> Result<Tensor> {
    let dimensions = input_dimensions(shape, layout, image)?;
    let pixels = oracle_resized_rgb(image, &dimensions, normalization);
    let values = oracle_populate_tensor(&pixels, &dimensions, layout);
    let tensor_shape = match layout {
        OnnxInputLayout::Nchw | OnnxInputLayout::Auto => vec![
            dimensions.batch,
            dimensions.channels,
            dimensions.height,
            dimensions.width,
        ],
        OnnxInputLayout::Nhwc => vec![
            dimensions.batch,
            dimensions.height,
            dimensions.width,
            dimensions.channels,
        ],
    };
    Tensor::from_shape(&tensor_shape, &values).map_err(super::super::execution::onnx_error)
}

fn oracle_resized_rgb(
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    normalization: OnnxNormalization,
) -> Vec<f32> {
    let source = oracle_rgb_pixels(image, normalization);
    let mut resized = Vec::with_capacity(dimensions.image_elements() * 3);
    for target_index in 0..dimensions.image_elements() {
        let base = source_index(target_index, image, dimensions) * 3;
        resized.extend_from_slice(&source[base..base + 3]);
    }
    resized
}

fn oracle_rgb_pixels(image: &ImageBuffer, normalization: OnnxNormalization) -> Vec<f32> {
    (0..image.width as usize * image.height as usize)
        .flat_map(|index| normalized_rgb(image, index, normalization))
        .collect()
}

fn oracle_populate_tensor(
    pixels: &[f32],
    dimensions: &InputDimensions,
    layout: OnnxInputLayout,
) -> Vec<f32> {
    let mut tensor = Vec::with_capacity(dimensions.batch * dimensions.batch_elements());
    for _ in 0..dimensions.batch {
        match layout {
            OnnxInputLayout::Nchw | OnnxInputLayout::Auto => {
                oracle_append_nchw(&mut tensor, pixels, dimensions.channels);
            }
            OnnxInputLayout::Nhwc => {
                for pixel in pixels.chunks_exact(3) {
                    tensor.extend_from_slice(&pixel[..dimensions.channels]);
                }
            }
        }
    }
    tensor
}

fn oracle_append_nchw(tensor: &mut Vec<f32>, pixels: &[f32], channels: usize) {
    for channel in 0..channels {
        for pixel in pixels.chunks_exact(3) {
            tensor.push(if channels == 1 {
                0.299 * pixel[0] + 0.587 * pixel[1] + 0.114 * pixel[2]
            } else {
                pixel[channel]
            });
        }
    }
}

fn assert_matches_oracle(
    image: &ImageBuffer,
    shape: &[usize],
    layout: OnnxInputLayout,
    normalization: OnnxNormalization,
) {
    let actual = image_tensor(image, shape, layout, normalization).unwrap();
    let expected = oracle_image_tensor(image, shape, layout, normalization).unwrap();
    assert_eq!(actual.shape(), expected.shape());
    let actual_plain = actual.try_as_plain().unwrap();
    let expected_plain = expected.try_as_plain().unwrap();
    assert_eq!(
        actual_plain.as_slice::<f32>().unwrap(),
        expected_plain.as_slice::<f32>().unwrap()
    );
}

fn shape(layout: OnnxInputLayout, batch: usize, channels: usize) -> [usize; 4] {
    match layout {
        OnnxInputLayout::Nchw | OnnxInputLayout::Auto => [batch, channels, 7, 5],
        OnnxInputLayout::Nhwc => [batch, 7, 5, channels],
    }
}

fn matrix_case(index: usize) -> (OnnxInputLayout, usize, OnnxNormalization, usize) {
    let layouts = [
        OnnxInputLayout::Auto,
        OnnxInputLayout::Nchw,
        OnnxInputLayout::Nhwc,
    ];
    let normalizations = [OnnxNormalization::ZeroToOne, OnnxNormalization::None];
    (
        layouts[index / 8],
        [1, 3][index / 4 % 2],
        normalizations[index / 2 % 2],
        [1, 3][index % 2],
    )
}

#[test]
fn direct_tensor_matches_seeded_baseline_matrix() {
    let seed = 1_592_594_996;
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = fixture(17, 11, format, seed ^ format.bpp() as u64);
        for index in 0..24 {
            let (layout, channels, normalization, batch) = matrix_case(index);
            assert_matches_oracle(
                &image,
                &shape(layout, batch, channels),
                layout,
                normalization,
            );
        }
    }
}

#[test]
fn large_source_small_target_matches_baseline() {
    let image = fixture(1_001, 999, PixelFormat::Rgba8, 0xD1CE_CAFE);
    assert_matches_oracle(
        &image,
        &[4, 3, 2, 5],
        OnnxInputLayout::Nchw,
        OnnxNormalization::ZeroToOne,
    );
    assert_matches_oracle(
        &image,
        &[4, 2, 5, 1],
        OnnxInputLayout::Nhwc,
        OnnxNormalization::None,
    );
}

#[test]
fn one_channel_layout_quirk_is_preserved() {
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![100, 50, 25]).unwrap();
    let nchw = image_tensor(
        &image,
        &[1, 1, 1, 1],
        OnnxInputLayout::Nchw,
        OnnxNormalization::None,
    )
    .unwrap();
    let nhwc = image_tensor(
        &image,
        &[1, 1, 1, 1],
        OnnxInputLayout::Nhwc,
        OnnxNormalization::None,
    )
    .unwrap();
    assert_eq!(
        tensor_values(&nchw),
        &[0.299 * 100.0 + 0.587 * 50.0 + 0.114 * 25.0]
    );
    assert_eq!(tensor_values(&nhwc), &[100.0]);
}

fn tensor_values(tensor: &Tensor) -> &[f32] {
    tensor.try_as_plain().unwrap().as_slice::<f32>().unwrap()
}

fn new_allocations(image: &ImageBuffer, shape: &[usize]) -> AllocationSample {
    measure_allocations(|| {
        image_tensor(image, shape, OnnxInputLayout::Nchw, OnnxNormalization::None).unwrap()
    })
    .1
}

fn old_allocations(image: &ImageBuffer, shape: &[usize]) -> AllocationSample {
    measure_allocations(|| {
        oracle_image_tensor(image, shape, OnnxInputLayout::Nchw, OnnxNormalization::None).unwrap()
    })
    .1
}

#[test]
fn direct_allocation_is_independent_of_source_size() {
    let small = fixture(31, 29, PixelFormat::Rgb8, 7);
    let large = fixture(1_000, 1_000, PixelFormat::Rgb8, 7);
    let shape = [2, 3, 7, 5];
    let new_small = new_allocations(&small, &shape);
    let new_large = new_allocations(&large, &shape);
    let old_small = old_allocations(&small, &shape);
    let old_large = old_allocations(&large, &shape);
    assert_eq!(new_small, new_large);
    assert_eq!(
        new_large,
        AllocationSample {
            allocations: 1,
            bytes: 2 * 3 * 7 * 5 * std::mem::size_of::<f32>(),
        }
    );
    assert_eq!(old_large.allocations, 5);
    assert_eq!(
        old_large.bytes - old_small.bytes,
        (1_000_000 - 31 * 29) * 3 * 4
    );
    assert!(new_large.bytes < old_large.bytes);
}

#[test]
fn layout_resolution_and_validation_limits_are_unchanged() {
    assert_eq!(
        resolve_layout(&[1, 9, 7, 3], OnnxInputLayout::Auto),
        OnnxInputLayout::Nhwc
    );
    assert_eq!(
        resolve_layout(&[1, 3, 9, 7], OnnxInputLayout::Auto),
        OnnxInputLayout::Nchw
    );
    let image = fixture(3, 2, PixelFormat::Gray8, 11);
    assert!(image_tensor(
        &image,
        &[1, 2, 2, 3],
        OnnxInputLayout::Nchw,
        OnnxNormalization::None
    )
    .is_err());
    assert!(image_tensor(
        &image,
        &[17, 1, 2, 3],
        OnnxInputLayout::Nchw,
        OnnxNormalization::None
    )
    .is_err());
    assert!(image_tensor(
        &image,
        &[1, 3, 4096, 4096],
        OnnxInputLayout::Nchw,
        OnnxNormalization::None
    )
    .is_err());
    assert!(image_tensor(
        &image,
        &[1, 3, 2],
        OnnxInputLayout::Nchw,
        OnnxNormalization::None
    )
    .is_err());
}
