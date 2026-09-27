use super::input::{image_tensor, resolve_layout};
use super::types::{OnnxInferenceOptions, OnnxOutputSummary, OnnxReport};
use super::validation::{
    self, MAX_ONNX_AGGREGATE_TENSOR_BYTES, MAX_ONNX_NODES, MAX_ONNX_OUTPUTS, MAX_ONNX_TENSOR_BYTES,
    MAX_ONNX_TENSOR_ELEMENTS, MAX_ONNX_TENSOR_RANK,
};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use std::path::Path;
use tract_onnx::prelude::*;

pub(super) fn onnx_error(error: impl std::fmt::Display) -> ScanError {
    ScanError::Other(format!("ONNX inference failed: {error}"))
}

pub(super) fn run(
    image: &ImageBuffer,
    path: &Path,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    let model = validation::load_model(path)?;
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

struct ModelInput {
    name: String,
    shape: Vec<usize>,
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
    let typed = bind_model_input(model, &tensor)?;
    validate_typed_model(&typed)?;
    let optimized = optimize_model(typed)?;
    validate_typed_model(&optimized)?;
    let outputs = execute_model(optimized, tensor)?;
    validate_runtime_outputs(&outputs)?;
    outputs
        .into_iter()
        .map(|output| output_summary(output.into_tensor()))
        .collect()
}

fn bind_model_input(model: InferenceModel, tensor: &Tensor) -> Result<TypedModel> {
    model
        .with_input_fact(0, TypedFact::shape_and_dt_of(tensor).into())
        .map_err(onnx_error)?
        .into_typed()
        .map_err(onnx_error)
}

fn optimize_model(model: TypedModel) -> Result<TypedModel> {
    model.into_optimized().map_err(onnx_error)
}

fn execute_model(model: TypedModel, tensor: Tensor) -> Result<TVec<TValue>> {
    model
        .into_runnable()
        .map_err(onnx_error)?
        .run(tvec!(tensor.into_tvalue()))
        .map_err(onnx_error)
}

fn validate_runtime_outputs(outputs: &TVec<TValue>) -> Result<()> {
    if outputs.len() > MAX_ONNX_OUTPUTS {
        return Err(ScanError::Unsupported(
            "ONNX runtime returned too many outputs".into(),
        ));
    }
    let mut aggregate = 0usize;
    for output in outputs {
        aggregate = checked_runtime_aggregate(aggregate, runtime_output_bytes(output)?)?;
    }
    Ok(())
}

fn runtime_output_bytes(output: &TValue) -> Result<usize> {
    output
        .len()
        .checked_mul(output.datum_type().size_of())
        .ok_or_else(|| ScanError::Invalid("ONNX output size overflow".into()))
}

fn checked_runtime_aggregate(aggregate: usize, bytes: usize) -> Result<usize> {
    let aggregate = aggregate
        .checked_add(bytes)
        .ok_or_else(|| ScanError::Invalid("ONNX output aggregate overflow".into()))?;
    if bytes > MAX_ONNX_TENSOR_BYTES || aggregate > MAX_ONNX_AGGREGATE_TENSOR_BYTES {
        return Err(ScanError::Unsupported(
            "ONNX runtime output exceeds the safety limit".into(),
        ));
    }
    Ok(aggregate)
}

fn tensor_bytes(tensor: &Tensor) -> Result<usize> {
    tensor
        .len()
        .checked_mul(tensor.datum_type().size_of())
        .ok_or_else(|| ScanError::Invalid("ONNX output size overflow".into()))
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
            aggregate = validate_typed_output(output, aggregate)?;
        }
    }
    Ok(())
}

fn validate_typed_output(output: &Outlet<TypedFact>, aggregate: usize) -> Result<usize> {
    let shape = concrete_output_shape(output)?;
    validate_output_rank(shape)?;
    let elements = checked_output_elements(shape)?;
    let bytes = checked_output_bytes(elements, output.fact.datum_type)?;
    let aggregate = checked_output_aggregate(aggregate, bytes)?;
    validate_output_limits(elements, bytes, aggregate)?;
    Ok(aggregate)
}

fn concrete_output_shape(output: &Outlet<TypedFact>) -> Result<&[usize]> {
    output.fact.shape.as_concrete().ok_or_else(|| {
        ScanError::Unsupported(
            "ONNX graph retains a dynamic intermediate shape after input binding".into(),
        )
    })
}

fn validate_output_rank(shape: &[usize]) -> Result<()> {
    if shape.len() > MAX_ONNX_TENSOR_RANK {
        return Err(ScanError::Unsupported(
            "ONNX intermediate tensor rank exceeds the safety limit".into(),
        ));
    }
    Ok(())
}

fn checked_output_elements(shape: &[usize]) -> Result<usize> {
    shape.iter().try_fold(1usize, |total, dimension| {
        total
            .checked_mul(*dimension)
            .ok_or_else(|| ScanError::Invalid("ONNX intermediate size overflow".into()))
    })
}

fn checked_output_bytes(elements: usize, datum_type: DatumType) -> Result<usize> {
    elements
        .checked_mul(datum_type.size_of())
        .ok_or_else(|| ScanError::Invalid("ONNX intermediate byte size overflow".into()))
}

fn checked_output_aggregate(aggregate: usize, bytes: usize) -> Result<usize> {
    aggregate
        .checked_add(bytes)
        .ok_or_else(|| ScanError::Invalid("ONNX graph byte aggregate overflow".into()))
}

fn validate_output_limits(elements: usize, bytes: usize, aggregate: usize) -> Result<()> {
    if elements > MAX_ONNX_TENSOR_ELEMENTS
        || bytes > MAX_ONNX_TENSOR_BYTES
        || aggregate > MAX_ONNX_AGGREGATE_TENSOR_BYTES
    {
        return Err(ScanError::Unsupported(
            "ONNX intermediate tensors exceed the safety limit".into(),
        ));
    }
    Ok(())
}

fn output_summary(tensor: Tensor) -> Result<OnnxOutputSummary> {
    let bytes = tensor_bytes(&tensor)?;
    if tensor.len() > MAX_ONNX_TENSOR_ELEMENTS || bytes > MAX_ONNX_TENSOR_BYTES {
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
