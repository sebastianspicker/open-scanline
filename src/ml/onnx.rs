use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tract_onnx::prelude::*;

const MAX_ONNX_MODEL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ONNX_INPUT_ELEMENTS: usize = 16 * 1024 * 1024;
const MAX_ONNX_BATCH: usize = 16;
const MAX_ONNX_NODES: usize = 4_096;
const MAX_ONNX_OUTPUTS: usize = 64;
const MAX_ONNX_TENSOR_RANK: usize = 8;
const MAX_ONNX_TENSOR_ELEMENTS: usize = 64 * 1024 * 1024;
const MAX_ONNX_TENSOR_BYTES: usize = 256 * 1024 * 1024;
const MAX_ONNX_AGGREGATE_TENSOR_BYTES: usize = 512 * 1024 * 1024;

/// Tensor arrangement used for a user supplied image model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnnxInputLayout {
    /// Infer the layout from the model's static channel axis; defaults to NCHW.
    #[default]
    Auto,
    Nchw,
    Nhwc,
}

/// Pixel conversion performed before inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnnxNormalization {
    /// Convert 8-bit source samples to `0.0..=1.0` floating point values.
    #[default]
    ZeroToOne,
    /// Preserve 8-bit sample magnitudes in a floating point tensor.
    None,
}

/// Explicit user-model input policy: first model input, auto layout, and RGB
/// values in `0..1` by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OnnxInferenceOptions {
    pub input_name: Option<String>,
    pub layout: OnnxInputLayout,
    pub normalization: OnnxNormalization,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxOutputSummary {
    pub shape: Vec<usize>,
    pub datum_type: String,
    pub size: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub sample: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxReport {
    pub ok: bool,
    pub engine: String,
    pub model: PathBuf,
    pub input_name: String,
    pub input_shape: Vec<usize>,
    pub layout: OnnxInputLayout,
    pub normalization: OnnxNormalization,
    pub outputs: Vec<OnnxOutputSummary>,
}

impl OnnxReport {
    pub fn as_dict(&self) -> Value {
        serde_json::to_value(self).expect("ONNX inference report is serializable")
    }
}

#[derive(Debug)]
struct InputDimensions {
    batch: usize,
    channels: usize,
    height: usize,
    width: usize,
}

impl InputDimensions {
    fn tensor_shape(&self, layout: OnnxInputLayout) -> Vec<usize> {
        match layout {
            OnnxInputLayout::Nchw | OnnxInputLayout::Auto => {
                vec![self.batch, self.channels, self.height, self.width]
            }
            OnnxInputLayout::Nhwc => vec![self.batch, self.height, self.width, self.channels],
        }
    }
}

struct ModelInput {
    name: String,
    shape: Vec<usize>,
}

fn onnx_error(error: impl std::fmt::Display) -> ScanError {
    ScanError::Other(format!("ONNX inference failed: {error}"))
}

fn model_dimension(shape: &[usize], axis: usize, fallback: usize) -> usize {
    shape
        .get(axis)
        .copied()
        .filter(|dimension| *dimension > 0)
        .unwrap_or(fallback)
}

fn resolve_layout(shape: &[usize], requested: OnnxInputLayout) -> OnnxInputLayout {
    match requested {
        OnnxInputLayout::Auto if shape.len() == 4 && matches!(shape.get(3), Some(1 | 3)) => {
            OnnxInputLayout::Nhwc
        }
        OnnxInputLayout::Auto => OnnxInputLayout::Nchw,
        layout => layout,
    }
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
    let dimensions = match layout {
        OnnxInputLayout::Nchw | OnnxInputLayout::Auto => InputDimensions {
            batch: model_dimension(shape, 0, 1),
            channels: model_dimension(shape, 1, 3),
            height: model_dimension(shape, 2, image.height as usize),
            width: model_dimension(shape, 3, image.width as usize),
        },
        OnnxInputLayout::Nhwc => InputDimensions {
            batch: model_dimension(shape, 0, 1),
            channels: model_dimension(shape, 3, 3),
            height: model_dimension(shape, 1, image.height as usize),
            width: model_dimension(shape, 2, image.width as usize),
        },
    };
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
    let tensor_elements = dimensions
        .batch
        .checked_mul(dimensions.channels)
        .and_then(|value| value.checked_mul(dimensions.height))
        .and_then(|value| value.checked_mul(dimensions.width))
        .ok_or_else(|| ScanError::Invalid("ONNX input dimensions overflow".into()))?;
    let resized_elements = dimensions
        .height
        .checked_mul(dimensions.width)
        .and_then(|value| value.checked_mul(3))
        .ok_or_else(|| ScanError::Invalid("ONNX image-resize dimensions overflow".into()))?;
    if tensor_elements > MAX_ONNX_INPUT_ELEMENTS || resized_elements > MAX_ONNX_INPUT_ELEMENTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX image input exceeds the {MAX_ONNX_INPUT_ELEMENTS}-element safety limit"
        )));
    }
    Ok(dimensions)
}

fn normalize(sample: u8, normalization: OnnxNormalization) -> f32 {
    match normalization {
        OnnxNormalization::ZeroToOne => sample as f32 / 255.0,
        OnnxNormalization::None => sample as f32,
    }
}

fn rgb_pixels(image: &ImageBuffer, normalization: OnnxNormalization) -> Vec<f32> {
    match image.pixel_format {
        PixelFormat::Gray8 => image
            .data
            .iter()
            .flat_map(|&sample| [normalize(sample, normalization); 3])
            .collect(),
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => (0..(image.width * image.height) as usize)
            .flat_map(|index| {
                let base = index * image.bpp();
                [
                    normalize(image.data[base], normalization),
                    normalize(image.data[base + 1], normalization),
                    normalize(image.data[base + 2], normalization),
                ]
            })
            .collect(),
    }
}

fn resized_rgb(
    image: &ImageBuffer,
    dimensions: &InputDimensions,
    normalization: OnnxNormalization,
) -> Vec<f32> {
    let source = rgb_pixels(image, normalization);
    let source_width = image.width as usize;
    let source_height = image.height as usize;
    let mut resized = Vec::with_capacity(dimensions.width * dimensions.height * 3);
    for y in 0..dimensions.height {
        let source_y = (y * source_height / dimensions.height).min(source_height.saturating_sub(1));
        for x in 0..dimensions.width {
            let source_x =
                (x * source_width / dimensions.width).min(source_width.saturating_sub(1));
            let base = (source_y * source_width + source_x) * 3;
            resized.extend_from_slice(&source[base..base + 3]);
        }
    }
    resized
}

fn append_nchw_batch(tensor: &mut Vec<f32>, pixels: &[f32], channels: usize) {
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

fn populate_tensor(
    pixels: &[f32],
    dimensions: &InputDimensions,
    layout: OnnxInputLayout,
) -> Vec<f32> {
    let mut tensor = Vec::with_capacity(
        dimensions.batch * dimensions.channels * dimensions.height * dimensions.width,
    );
    for _ in 0..dimensions.batch {
        match layout {
            OnnxInputLayout::Nchw | OnnxInputLayout::Auto => {
                append_nchw_batch(&mut tensor, pixels, dimensions.channels)
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

fn image_tensor(
    image: &ImageBuffer,
    shape: &[usize],
    layout: OnnxInputLayout,
    normalization: OnnxNormalization,
) -> Result<Tensor> {
    let dimensions = input_dimensions(shape, layout, image)?;
    let pixels = resized_rgb(image, &dimensions, normalization);
    let values = populate_tensor(&pixels, &dimensions, layout);
    Tensor::from_shape(&dimensions.tensor_shape(layout), &values).map_err(onnx_error)
}

type NumericSummary = (Option<f64>, Option<f64>, Option<f64>, Vec<f64>);

fn numeric_summary(tensor: &Tensor) -> Result<NumericSummary> {
    if tensor.datum_type() != f32::datum_type() {
        return Ok((None, None, None, Vec::new()));
    }
    let view = tensor
        .try_as_plain()
        .and_then(|plain| plain.to_array_view::<f32>())
        .map_err(onnx_error)?;
    let sample = view
        .iter()
        .take(64)
        .map(|value| *value as f64)
        .collect::<Vec<_>>();
    if view.is_empty() {
        return Ok((Some(0.0), Some(0.0), Some(0.0), sample));
    }
    let min = view.iter().copied().fold(f32::INFINITY, f32::min) as f64;
    let max = view.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mean = view.iter().map(|value| *value as f64).sum::<f64>() / view.len() as f64;
    Ok((Some(min), Some(max), Some(mean), sample))
}

fn output_summary(tensor: Tensor) -> Result<OnnxOutputSummary> {
    if tensor.len() > MAX_ONNX_TENSOR_ELEMENTS
        || tensor
            .len()
            .checked_mul(tensor.datum_type().size_of())
            .is_none_or(|bytes| bytes > MAX_ONNX_TENSOR_BYTES)
    {
        return Err(ScanError::Unsupported(
            "ONNX output tensor exceeds the safety limit".into(),
        ));
    }
    let (min, max, mean, sample) = numeric_summary(&tensor)?;
    Ok(OnnxOutputSummary {
        shape: tensor.shape().to_vec(),
        datum_type: format!("{:?}", tensor.datum_type()),
        size: tensor.len(),
        min,
        max,
        mean,
        sample,
    })
}

fn load_model(path: &Path) -> Result<InferenceModel> {
    let framework = tract_onnx::onnx();
    let proto = framework.proto_model_for_path(path).map_err(onnx_error)?;
    validate_model_proto(&proto)?;
    framework.model_for_proto_model(&proto).map_err(onnx_error)
}

#[derive(Default)]
struct ProtoBudget {
    nodes: usize,
    tensor_bytes: usize,
}

fn validate_model_proto(proto: &tract_onnx::pb::ModelProto) -> Result<()> {
    if !proto.training_info.is_empty() {
        return Err(ScanError::Unsupported(
            "ONNX training graphs are not accepted for image inference".into(),
        ));
    }
    let graph = proto
        .graph
        .as_ref()
        .ok_or_else(|| ScanError::Invalid("ONNX model has no graph".into()))?;
    let mut budget = ProtoBudget::default();
    validate_proto_graph(graph, &mut budget)?;
    for function in &proto.functions {
        budget.nodes = budget
            .nodes
            .checked_add(function.node.len())
            .ok_or_else(|| ScanError::Invalid("ONNX node count overflow".into()))?;
        validate_node_budget(&function.node, &mut budget)?;
    }
    if budget.nodes > MAX_ONNX_NODES {
        return Err(ScanError::Unsupported(format!(
            "ONNX graph exceeds the {MAX_ONNX_NODES}-node safety limit"
        )));
    }
    Ok(())
}

fn validate_proto_graph(
    graph: &tract_onnx::pb::GraphProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    budget.nodes = budget
        .nodes
        .checked_add(graph.node.len())
        .ok_or_else(|| ScanError::Invalid("ONNX node count overflow".into()))?;
    if budget.nodes > MAX_ONNX_NODES {
        return Err(ScanError::Unsupported(format!(
            "ONNX graph exceeds the {MAX_ONNX_NODES}-node safety limit"
        )));
    }
    if graph.output.len() > MAX_ONNX_OUTPUTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX graph exceeds the {MAX_ONNX_OUTPUTS}-output safety limit"
        )));
    }
    for value in graph
        .input
        .iter()
        .chain(&graph.output)
        .chain(&graph.value_info)
    {
        validate_value_info(value)?;
    }
    for tensor in &graph.initializer {
        validate_tensor_proto(tensor, budget)?;
    }
    for sparse in &graph.sparse_initializer {
        validate_sparse_proto(sparse, budget)?;
    }
    validate_node_budget(&graph.node, budget)
}

fn validate_value_info(value: &tract_onnx::pb::ValueInfoProto) -> Result<()> {
    use tract_onnx::pb::tensor_shape_proto::dimension::Value as DimensionValue;
    use tract_onnx::pb::type_proto::Value as TypeValue;

    let Some(TypeValue::TensorType(tensor)) = value
        .r#type
        .as_ref()
        .and_then(|value_type| value_type.value.as_ref())
    else {
        return Ok(());
    };
    let Some(shape) = tensor.shape.as_ref() else {
        return Ok(());
    };
    if shape.dim.len() > MAX_ONNX_TENSOR_RANK {
        return Err(ScanError::Unsupported(format!(
            "ONNX tensor rank exceeds the {MAX_ONNX_TENSOR_RANK}-axis safety limit"
        )));
    }
    let concrete = shape
        .dim
        .iter()
        .map(|dimension| match dimension.value.as_ref() {
            Some(DimensionValue::DimValue(value)) => Some(*value),
            Some(DimensionValue::DimParam(_)) | None => None,
        })
        .collect::<Option<Vec<_>>>();
    if let Some(dimensions) = concrete {
        let elements = checked_proto_elements(&dimensions)?;
        let bytes = elements
            .checked_mul(tensor_element_bytes(tensor.elem_type)?)
            .ok_or_else(|| ScanError::Invalid("ONNX tensor byte size overflow".into()))?;
        if bytes > MAX_ONNX_TENSOR_BYTES {
            return Err(ScanError::Unsupported(
                "ONNX declared tensor exceeds the per-tensor safety limit".into(),
            ));
        }
    }
    Ok(())
}

fn validate_node_budget(
    nodes: &[tract_onnx::pb::NodeProto],
    budget: &mut ProtoBudget,
) -> Result<()> {
    for node in nodes {
        for attribute in &node.attribute {
            if let Some(tensor) = &attribute.t {
                validate_tensor_proto(tensor, budget)?;
            }
            for tensor in &attribute.tensors {
                validate_tensor_proto(tensor, budget)?;
            }
            if let Some(sparse) = &attribute.sparse_tensor {
                validate_sparse_proto(sparse, budget)?;
            }
            for sparse in &attribute.sparse_tensors {
                validate_sparse_proto(sparse, budget)?;
            }
            if let Some(graph) = &attribute.g {
                validate_proto_graph(graph, budget)?;
            }
            for graph in &attribute.graphs {
                validate_proto_graph(graph, budget)?;
            }
        }
    }
    Ok(())
}

fn validate_sparse_proto(
    sparse: &tract_onnx::pb::SparseTensorProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    checked_proto_elements(&sparse.dims)?;
    if let Some(values) = &sparse.values {
        validate_tensor_proto(values, budget)?;
    }
    if let Some(indices) = &sparse.indices {
        validate_tensor_proto(indices, budget)?;
    }
    Ok(())
}

fn validate_tensor_proto(
    tensor: &tract_onnx::pb::TensorProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    if !tensor.external_data.is_empty()
        || tensor.data_location == Some(tract_onnx::pb::tensor_proto::DataLocation::External as i32)
    {
        return Err(ScanError::Unsupported(
            "ONNX external tensor data is not accepted".into(),
        ));
    }
    let elements = checked_proto_elements(&tensor.dims)?;
    let encoded = tensor
        .raw_data
        .len()
        .checked_add(
            tensor
                .float_data
                .len()
                .saturating_mul(std::mem::size_of::<f32>()),
        )
        .and_then(|value| {
            value.checked_add(
                tensor
                    .int32_data
                    .len()
                    .saturating_mul(std::mem::size_of::<i32>()),
            )
        })
        .and_then(|value| {
            value.checked_add(
                tensor
                    .int64_data
                    .len()
                    .saturating_mul(std::mem::size_of::<i64>()),
            )
        })
        .and_then(|value| {
            value.checked_add(
                tensor
                    .double_data
                    .len()
                    .saturating_mul(std::mem::size_of::<f64>()),
            )
        })
        .and_then(|value| {
            value.checked_add(
                tensor
                    .uint64_data
                    .len()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
        })
        .and_then(|value| {
            tensor
                .string_data
                .iter()
                .try_fold(value, |sum, string| sum.checked_add(string.len()))
        })
        .ok_or_else(|| ScanError::Invalid("ONNX initializer size overflow".into()))?;
    let declared = elements
        .checked_mul(tensor_element_bytes(tensor.data_type)?)
        .ok_or_else(|| ScanError::Invalid("ONNX initializer byte size overflow".into()))?;
    let accounted = encoded.max(declared);
    if accounted > MAX_ONNX_TENSOR_BYTES {
        return Err(ScanError::Unsupported(
            "ONNX initializer exceeds the per-tensor safety limit".into(),
        ));
    }
    budget.tensor_bytes = budget
        .tensor_bytes
        .checked_add(accounted)
        .ok_or_else(|| ScanError::Invalid("ONNX initializer aggregate overflow".into()))?;
    if budget.tensor_bytes > MAX_ONNX_AGGREGATE_TENSOR_BYTES {
        return Err(ScanError::Unsupported(
            "ONNX initializers exceed the aggregate safety limit".into(),
        ));
    }
    Ok(())
}

fn tensor_element_bytes(data_type: i32) -> Result<usize> {
    use tract_onnx::pb::tensor_proto::DataType;

    let data_type = DataType::try_from(data_type)
        .map_err(|_| ScanError::Invalid("ONNX tensor has an unknown data type".into()))?;
    match data_type {
        DataType::Undefined => Err(ScanError::Invalid(
            "ONNX tensor has an undefined data type".into(),
        )),
        DataType::Uint8
        | DataType::Int8
        | DataType::Bool
        | DataType::Float8e4m3fn
        | DataType::Float8e4m3fnuz
        | DataType::Float8e5m2
        | DataType::Float8e5m2fnuz
        | DataType::Uint4
        | DataType::Int4
        | DataType::Float4e2m1 => Ok(1),
        DataType::Uint16 | DataType::Int16 | DataType::Float16 | DataType::Bfloat16 => Ok(2),
        DataType::Float | DataType::Int32 | DataType::Uint32 => Ok(4),
        DataType::Int64 | DataType::Double | DataType::Uint64 | DataType::Complex64 => Ok(8),
        DataType::Complex128 | DataType::String => Ok(16),
    }
}

fn checked_proto_elements(dims: &[i64]) -> Result<usize> {
    if dims.len() > MAX_ONNX_TENSOR_RANK {
        return Err(ScanError::Unsupported(format!(
            "ONNX tensor rank exceeds the {MAX_ONNX_TENSOR_RANK}-axis safety limit"
        )));
    }
    let elements = dims.iter().try_fold(1usize, |product, &dimension| {
        let dimension = usize::try_from(dimension)
            .map_err(|_| ScanError::Invalid("ONNX tensor has a negative dimension".into()))?;
        product
            .checked_mul(dimension)
            .ok_or_else(|| ScanError::Invalid("ONNX tensor dimensions overflow".into()))
    })?;
    if elements > MAX_ONNX_TENSOR_ELEMENTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX tensor exceeds the {MAX_ONNX_TENSOR_ELEMENTS}-element safety limit"
        )));
    }
    Ok(elements)
}

fn prepare_model_input(
    model: &InferenceModel,
    image: &ImageBuffer,
    requested_name: Option<&str>,
) -> Result<ModelInput> {
    let input_outlets = model.input_outlets().map_err(onnx_error)?;
    if input_outlets.len() != 1 {
        return Err(ScanError::Unsupported(format!(
            "run_user_onnx supports exactly one model input, found {}",
            input_outlets.len()
        )));
    }
    let name = model.node(input_outlets[0].node).name.to_string();
    if let Some(requested) = requested_name.filter(|requested| *requested != name) {
        return Err(ScanError::Invalid(format!(
            "ONNX input '{requested}' not found; model input is '{name}'"
        )));
    }
    let shape = model
        .input_fact(0)
        .map_err(onnx_error)?
        .shape
        .as_concrete_finite()
        .map_err(onnx_error)?
        .unwrap_or_else(|| tvec!(1, 3, image.height as usize, image.width as usize))
        .into_iter()
        .collect();
    Ok(ModelInput { name, shape })
}

fn run_model(model: InferenceModel, tensor: Tensor) -> Result<Vec<OnnxOutputSummary>> {
    let typed = model
        .with_input_fact(0, TypedFact::shape_and_dt_of(&tensor).into())
        .map_err(onnx_error)?
        .into_typed()
        .map_err(onnx_error)?;
    validate_typed_model(&typed)?;
    let optimized = typed.into_optimized().map_err(onnx_error)?;
    validate_typed_model(&optimized)?;
    let runnable = optimized.into_runnable().map_err(onnx_error)?;
    let outputs = runnable
        .run(tvec!(tensor.into_tvalue()))
        .map_err(onnx_error)?;
    if outputs.len() > MAX_ONNX_OUTPUTS {
        return Err(ScanError::Unsupported(
            "ONNX runtime returned too many outputs".into(),
        ));
    }
    let mut aggregate = 0usize;
    for output in &outputs {
        let bytes = output
            .len()
            .checked_mul(output.datum_type().size_of())
            .ok_or_else(|| ScanError::Invalid("ONNX output size overflow".into()))?;
        aggregate = aggregate
            .checked_add(bytes)
            .ok_or_else(|| ScanError::Invalid("ONNX output aggregate overflow".into()))?;
        if bytes > MAX_ONNX_TENSOR_BYTES || aggregate > MAX_ONNX_AGGREGATE_TENSOR_BYTES {
            return Err(ScanError::Unsupported(
                "ONNX runtime output exceeds the safety limit".into(),
            ));
        }
    }
    outputs
        .into_iter()
        .map(|output| output_summary(output.into_tensor()))
        .collect()
}

fn validate_typed_model(model: &TypedModel) -> Result<()> {
    if model.nodes().len() > MAX_ONNX_NODES || model.outputs.len() > MAX_ONNX_OUTPUTS {
        return Err(ScanError::Unsupported(
            "typed ONNX graph exceeds the node/output safety limit".into(),
        ));
    }
    let mut aggregate = 0usize;
    for node in model.nodes() {
        for output in &node.outputs {
            let shape = output.fact.shape.as_concrete().ok_or_else(|| {
                ScanError::Unsupported(
                    "ONNX graph retains a dynamic intermediate shape after input binding".into(),
                )
            })?;
            if shape.len() > MAX_ONNX_TENSOR_RANK {
                return Err(ScanError::Unsupported(
                    "ONNX intermediate tensor rank exceeds the safety limit".into(),
                ));
            }
            let elements = shape.iter().try_fold(1usize, |product, dimension| {
                product
                    .checked_mul(*dimension)
                    .ok_or_else(|| ScanError::Invalid("ONNX intermediate size overflow".into()))
            })?;
            let bytes = elements
                .checked_mul(output.fact.datum_type.size_of())
                .ok_or_else(|| ScanError::Invalid("ONNX intermediate byte size overflow".into()))?;
            aggregate = aggregate
                .checked_add(bytes)
                .ok_or_else(|| ScanError::Invalid("ONNX graph byte aggregate overflow".into()))?;
            if elements > MAX_ONNX_TENSOR_ELEMENTS
                || bytes > MAX_ONNX_TENSOR_BYTES
                || aggregate > MAX_ONNX_AGGREGATE_TENSOR_BYTES
            {
                return Err(ScanError::Unsupported(
                    "ONNX intermediate tensors exceed the safety limit".into(),
                ));
            }
        }
    }
    Ok(())
}

fn onnx_model_path(path: &Path) -> Result<()> {
    if !path.is_file() {
        Err(ScanError::Invalid(format!(
            "ONNX model not found: {}",
            path.display()
        )))
    } else {
        let size = std::fs::metadata(path)?.len();
        if size > MAX_ONNX_MODEL_BYTES {
            return Err(ScanError::Unsupported(format!(
                "ONNX model is {size} bytes; the safety limit is {MAX_ONNX_MODEL_BYTES} bytes"
            )));
        }
        Ok(())
    }
}

/// Run a user-supplied ONNX model in a resource-contained worker process.
pub fn run_user_onnx(image: &ImageBuffer, model_path: impl AsRef<Path>) -> Result<OnnxReport> {
    run_user_onnx_with_options(image, model_path, &OnnxInferenceOptions::default())
}

pub fn run_user_onnx_with_options(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    let input = crate::backend_process::TemporaryOutput::new("onnx-input", "png")?;
    crate::imaging::save_image(input.path(), image, None, None)?;
    super::isolation::run_isolated_onnx(input.path(), model_path.as_ref(), options)
}

/// Run an ONNX model with an explicit Open Scanline worker executable.
///
/// Library hosts whose current executable is not `open-scanline` should use
/// this entry point. The worker must implement the private, version-matched
/// `__onnx-worker` command provided by the same Open Scanline build.
pub fn run_user_onnx_with_worker(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
    worker_executable: impl AsRef<Path>,
) -> Result<OnnxReport> {
    let input = crate::backend_process::TemporaryOutput::new("onnx-input", "png")?;
    crate::imaging::save_image(input.path(), image, None, None)?;
    super::isolation::run_isolated_onnx_with_executable(
        input.path(),
        model_path.as_ref(),
        options,
        worker_executable.as_ref(),
    )
}

/// Trusted-only in-process implementation used by the contained worker.
pub(crate) fn run_trusted_user_onnx_with_options(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    let path = model_path.as_ref();
    onnx_model_path(path)?;
    let model = load_model(path)?;
    let model_input = prepare_model_input(&model, image, options.input_name.as_deref())?;
    let layout = resolve_layout(&model_input.shape, options.layout);
    let tensor = image_tensor(image, &model_input.shape, layout, options.normalization)?;
    let input_shape = tensor.shape().to_vec();
    let outputs = run_model(model, tensor)?;
    Ok(OnnxReport {
        ok: true,
        engine: "tract-onnx".into(),
        model: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        input_name: model_input.name,
        input_shape,
        layout,
        normalization: options.normalization,
        outputs,
    })
}
