use super::types::{OnnxInputLayout, OnnxNormalization};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use tract_onnx::prelude::Tensor;

const MAX_ONNX_INPUT_ELEMENTS: usize = 16 * 1024 * 1024;
const MAX_ONNX_BATCH: usize = 16;

#[derive(Debug)]
struct InputDimensions {
    batch: usize,
    channels: usize,
    height: usize,
    width: usize,
}

impl InputDimensions {
    fn tensor_shape(&self, layout: OnnxInputLayout) -> [usize; 4] {
        match layout {
            OnnxInputLayout::Nchw | OnnxInputLayout::Auto => {
                [self.batch, self.channels, self.height, self.width]
            }
            OnnxInputLayout::Nhwc => [self.batch, self.height, self.width, self.channels],
        }
    }

    fn image_elements(&self) -> usize {
        self.height * self.width
    }

    fn batch_elements(&self) -> usize {
        self.channels * self.image_elements()
    }
}

pub(super) fn resolve_layout(shape: &[usize], requested: OnnxInputLayout) -> OnnxInputLayout {
    match requested {
        OnnxInputLayout::Auto if shape.len() == 4 && matches!(shape.get(3), Some(1 | 3)) => {
            OnnxInputLayout::Nhwc
        }
        OnnxInputLayout::Auto => OnnxInputLayout::Nchw,
        layout => layout,
    }
}

pub(super) fn image_tensor(
    image: &ImageBuffer,
    shape: &[usize],
    layout: OnnxInputLayout,
    normalization: OnnxNormalization,
) -> Result<Tensor> {
    let dimensions = input_dimensions(shape, layout, image)?;
    let tensor_shape = dimensions.tensor_shape(layout);
    let mut tensor = Tensor::zero::<f32>(&tensor_shape).map_err(super::execution::onnx_error)?;
    {
        let mut storage = tensor
            .try_as_plain_mut()
            .map_err(super::execution::onnx_error)?;
        let values = storage
            .as_slice_mut::<f32>()
            .map_err(super::execution::onnx_error)?;
        fill_tensor(values, image, &dimensions, layout, normalization);
    }
    Ok(tensor)
}

fn input_dimensions(
    shape: &[usize],
    layout: OnnxInputLayout,
    image: &ImageBuffer,
) -> Result<InputDimensions> {
    if shape.len() != 4 {
        return Err(ScanError::Unsupported(format!(
            "run_user_onnx requires a rank-4 image input, model declares rank {}",
            shape.len()
        )));
    }
    let dimensions = dimensions_for_layout(shape, layout, image);
    validate_dimensions(&dimensions)?;
    Ok(dimensions)
}

fn dimensions_for_layout(
    shape: &[usize],
    layout: OnnxInputLayout,
    image: &ImageBuffer,
) -> InputDimensions {
    let (height, width) = (image.height as usize, image.width as usize);
    match layout {
        OnnxInputLayout::Nchw | OnnxInputLayout::Auto => InputDimensions {
            batch: dimension(shape, 0, 1),
            channels: dimension(shape, 1, 3),
            height: dimension(shape, 2, height),
            width: dimension(shape, 3, width),
        },
        OnnxInputLayout::Nhwc => InputDimensions {
            batch: dimension(shape, 0, 1),
            channels: dimension(shape, 3, 3),
            height: dimension(shape, 1, height),
            width: dimension(shape, 2, width),
        },
    }
}

fn dimension(shape: &[usize], axis: usize, fallback: usize) -> usize {
    shape
        .get(axis)
        .copied()
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn validate_dimensions(dimensions: &InputDimensions) -> Result<()> {
    if !matches!(dimensions.channels, 1 | 3) {
        return Err(ScanError::Unsupported(format!(
            "run_user_onnx supports one or three input channels, model requires {}",
            dimensions.channels
        )));
    }
    if dimensions.batch > MAX_ONNX_BATCH {
        return Err(ScanError::Unsupported(format!(
            "run_user_onnx supports a batch dimension up to {MAX_ONNX_BATCH}, model requires {}",
            dimensions.batch
        )));
    }
    let tensor = product(
        &[
            dimensions.batch,
            dimensions.channels,
            dimensions.height,
            dimensions.width,
        ],
        "ONNX input",
    )?;
    let resized = product(
        &[dimensions.height, dimensions.width, 3],
        "ONNX image-resize",
    )?;
    if tensor > MAX_ONNX_INPUT_ELEMENTS || resized > MAX_ONNX_INPUT_ELEMENTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX image input exceeds the {MAX_ONNX_INPUT_ELEMENTS}-element safety limit"
        )));
    }
    Ok(())
}

fn product(values: &[usize], label: &str) -> Result<usize> {
    values.iter().try_fold(1usize, |total, value| {
        total
            .checked_mul(*value)
            .ok_or_else(|| ScanError::Invalid(format!("{label} dimensions overflow")))
    })
}

fn fill_tensor(
    output: &mut [f32],
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    layout: OnnxInputLayout,
    normalization: OnnxNormalization,
) {
    for batch in output.chunks_exact_mut(dimensions.batch_elements()) {
        match layout {
            OnnxInputLayout::Nchw | OnnxInputLayout::Auto => {
                fill_nchw_batch(batch, image, dimensions, normalization);
            }
            OnnxInputLayout::Nhwc => {
                fill_nhwc_batch(batch, image, dimensions, normalization);
            }
        }
    }
}

fn fill_nchw_batch(
    output: &mut [f32],
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    normalization: OnnxNormalization,
) {
    for (channel, plane) in output
        .chunks_exact_mut(dimensions.image_elements())
        .enumerate()
    {
        fill_nchw_plane(plane, image, dimensions, channel, normalization);
    }
}

fn fill_nchw_plane(
    output: &mut [f32],
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    channel: usize,
    normalization: OnnxNormalization,
) {
    for (target_index, value) in output.iter_mut().enumerate() {
        let source_index = source_index(target_index, image, dimensions);
        let pixel = normalized_rgb(image, source_index, normalization);
        *value = if dimensions.channels == 1 {
            0.299 * pixel[0] + 0.587 * pixel[1] + 0.114 * pixel[2]
        } else {
            pixel[channel]
        };
    }
}

fn fill_nhwc_batch(
    output: &mut [f32],
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    normalization: OnnxNormalization,
) {
    for (target_index, pixel_output) in output.chunks_exact_mut(dimensions.channels).enumerate() {
        let source_index = source_index(target_index, image, dimensions);
        let pixel = normalized_rgb(image, source_index, normalization);
        pixel_output.copy_from_slice(&pixel[..dimensions.channels]);
    }
}

fn source_index(target_index: usize, image: &ImageBuffer, dimensions: &InputDimensions) -> usize {
    let target_y = target_index / dimensions.width;
    let target_x = target_index % dimensions.width;
    let source_height = image.height as usize;
    let source_width = image.width as usize;
    let source_y =
        (target_y * source_height / dimensions.height).min(source_height.saturating_sub(1));
    let source_x = (target_x * source_width / dimensions.width).min(source_width.saturating_sub(1));
    source_y * source_width + source_x
}

fn normalized_rgb(
    image: &ImageBuffer,
    pixel_index: usize,
    normalization: OnnxNormalization,
) -> [f32; 3] {
    match image.pixel_format {
        PixelFormat::Gray8 => {
            let value = normalize(image.data[pixel_index], normalization);
            [value; 3]
        }
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
            let base = pixel_index * image.bpp();
            [
                normalize(image.data[base], normalization),
                normalize(image.data[base + 1], normalization),
                normalize(image.data[base + 2], normalization),
            ]
        }
    }
}

fn normalize(sample: u8, normalization: OnnxNormalization) -> f32 {
    match normalization {
        OnnxNormalization::ZeroToOne => sample as f32 / 255.0,
        OnnxNormalization::None => sample as f32,
    }
}

