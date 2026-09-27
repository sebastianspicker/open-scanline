use crate::error::{Result, ScanError};
use std::path::Path;
use tract_onnx::prelude::*;

pub(super) const MAX_ONNX_NODES: usize = 4_096;
pub(super) const MAX_ONNX_OUTPUTS: usize = 64;
pub(super) const MAX_ONNX_TENSOR_RANK: usize = 8;
pub(super) const MAX_ONNX_TENSOR_ELEMENTS: usize = 64 * 1024 * 1024;
pub(super) const MAX_ONNX_TENSOR_BYTES: usize = 256 * 1024 * 1024;
pub(super) const MAX_ONNX_AGGREGATE_TENSOR_BYTES: usize = 512 * 1024 * 1024;
const MAX_ONNX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct ProtoBudget {
    nodes: usize,
    tensor_bytes: usize,
}

pub(super) fn load_model(path: &Path) -> Result<InferenceModel> {
    validate_model_path(path)?;
    let framework = tract_onnx::onnx();
    let proto = framework
        .proto_model_for_path(path)
        .map_err(super::execution::onnx_error)?;
    validate_model_proto(&proto)?;
    framework
        .model_for_proto_model(&proto)
        .map_err(super::execution::onnx_error)
}

fn validate_model_path(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Err(ScanError::Invalid(format!(
            "ONNX model not found: {}",
            path.display()
        )));
    }
    let size = std::fs::metadata(path)?.len();
    if size > MAX_ONNX_MODEL_BYTES {
        return Err(ScanError::Unsupported(format!(
            "ONNX model is {size} bytes; the safety limit is {MAX_ONNX_MODEL_BYTES} bytes"
        )));
    }
    Ok(())
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
        add_nodes(&mut budget, function.node.len())?;
        validate_node_budget(&function.node, &mut budget)?;
    }
    validate_node_total(budget.nodes)
}

fn validate_proto_graph(
    graph: &tract_onnx::pb::GraphProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    add_nodes(budget, graph.node.len())?;
    validate_graph_output_count(graph.output.len())?;
    validate_graph_values(graph)?;
    validate_graph_tensors(graph, budget)?;
    validate_node_budget(&graph.node, budget)
}

fn validate_graph_output_count(output_count: usize) -> Result<()> {
    if output_count > MAX_ONNX_OUTPUTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX graph exceeds the {MAX_ONNX_OUTPUTS}-output safety limit"
        )));
    }
    Ok(())
}

fn validate_graph_values(graph: &tract_onnx::pb::GraphProto) -> Result<()> {
    for value in graph
        .input
        .iter()
        .chain(&graph.output)
        .chain(&graph.value_info)
    {
        validate_value_info(value)?;
    }
    Ok(())
}

fn validate_graph_tensors(
    graph: &tract_onnx::pb::GraphProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    for tensor in &graph.initializer {
        validate_tensor_proto(tensor, budget)?;
    }
    for sparse in &graph.sparse_initializer {
        validate_sparse_proto(sparse, budget)?;
    }
    Ok(())
}

fn add_nodes(budget: &mut ProtoBudget, count: usize) -> Result<()> {
    budget.nodes = budget
        .nodes
        .checked_add(count)
        .ok_or_else(|| ScanError::Invalid("ONNX node count overflow".into()))?;
    validate_node_total(budget.nodes)
}

fn validate_node_total(nodes: usize) -> Result<()> {
    if nodes > MAX_ONNX_NODES {
        Err(ScanError::Unsupported(format!(
            "ONNX graph exceeds the {MAX_ONNX_NODES}-node safety limit"
        )))
    } else {
        Ok(())
    }
}

fn validate_value_info(value: &tract_onnx::pb::ValueInfoProto) -> Result<()> {
    use tract_onnx::pb::tensor_shape_proto::dimension::Value as DimensionValue;
    use tract_onnx::pb::type_proto::Value as TypeValue;
    let Some(TypeValue::TensorType(tensor)) =
        value.r#type.as_ref().and_then(|kind| kind.value.as_ref())
    else {
        return Ok(());
    };
    let Some(shape) = tensor.shape.as_ref() else {
        return Ok(());
    };
    if shape.dim.len() > MAX_ONNX_TENSOR_RANK {
        return Err(rank_error());
    }
    let dimensions = shape
        .dim
        .iter()
        .map(|dimension| match dimension.value.as_ref() {
            Some(DimensionValue::DimValue(value)) => Some(*value),
            Some(DimensionValue::DimParam(_)) | None => None,
        })
        .collect::<Option<Vec<_>>>();
    if let Some(dimensions) = dimensions {
        validate_declared_tensor(&dimensions, tensor.elem_type)?;
    }
    Ok(())
}

fn validate_declared_tensor(dimensions: &[i64], data_type: i32) -> Result<()> {
    let bytes = checked_proto_elements(dimensions)?
        .checked_mul(tensor_element_bytes(data_type)?)
        .ok_or_else(|| ScanError::Invalid("ONNX tensor byte size overflow".into()))?;
    if bytes > MAX_ONNX_TENSOR_BYTES {
        return Err(ScanError::Unsupported(
            "ONNX declared tensor exceeds the per-tensor safety limit".into(),
        ));
    }
    Ok(())
}

fn validate_node_budget(
    nodes: &[tract_onnx::pb::NodeProto],
    budget: &mut ProtoBudget,
) -> Result<()> {
    for node in nodes {
        for attribute in &node.attribute {
            validate_attribute(attribute, budget)?;
        }
    }
    Ok(())
}

fn validate_attribute(
    attribute: &tract_onnx::pb::AttributeProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    validate_tensor_attributes(attribute, budget)?;
    validate_sparse_attributes(attribute, budget)?;
    validate_graph_attributes(attribute, budget)
}

fn validate_tensor_attributes(
    attribute: &tract_onnx::pb::AttributeProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    validate_optional_tensor(attribute.t.as_ref(), budget)?;
    for tensor in &attribute.tensors {
        validate_tensor_proto(tensor, budget)?;
    }
    Ok(())
}

fn validate_sparse_attributes(
    attribute: &tract_onnx::pb::AttributeProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    validate_optional_sparse(attribute.sparse_tensor.as_ref(), budget)?;
    for sparse in &attribute.sparse_tensors {
        validate_sparse_proto(sparse, budget)?;
    }
    Ok(())
}

fn validate_graph_attributes(
    attribute: &tract_onnx::pb::AttributeProto,
    budget: &mut ProtoBudget,
) -> Result<()> {
    validate_optional_graph(attribute.g.as_ref(), budget)?;
    for graph in &attribute.graphs {
        validate_proto_graph(graph, budget)?;
    }
    Ok(())
}

fn validate_optional_tensor(
    tensor: Option<&tract_onnx::pb::TensorProto>,
    budget: &mut ProtoBudget,
) -> Result<()> {
    tensor.map_or(Ok(()), |tensor| validate_tensor_proto(tensor, budget))
}

fn validate_optional_sparse(
    sparse: Option<&tract_onnx::pb::SparseTensorProto>,
    budget: &mut ProtoBudget,
) -> Result<()> {
    sparse.map_or(Ok(()), |sparse| validate_sparse_proto(sparse, budget))
}

fn validate_optional_graph(
    graph: Option<&tract_onnx::pb::GraphProto>,
    budget: &mut ProtoBudget,
) -> Result<()> {
    graph.map_or(Ok(()), |graph| validate_proto_graph(graph, budget))
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
    validate_embedded_tensor(tensor)?;
    let accounted = accounted_tensor_bytes(tensor)?;
    validate_tensor_byte_limit(accounted)?;
    add_tensor_bytes(budget, accounted)
}

fn validate_embedded_tensor(tensor: &tract_onnx::pb::TensorProto) -> Result<()> {
    let external_location = Some(tract_onnx::pb::tensor_proto::DataLocation::External as i32);
    if !tensor.external_data.is_empty() || tensor.data_location == external_location {
        return Err(ScanError::Unsupported(
            "ONNX external tensor data is not accepted".into(),
        ));
    }
    Ok(())
}

fn accounted_tensor_bytes(tensor: &tract_onnx::pb::TensorProto) -> Result<usize> {
    let declared = checked_proto_elements(&tensor.dims)?
        .checked_mul(tensor_element_bytes(tensor.data_type)?)
        .ok_or_else(|| ScanError::Invalid("ONNX initializer byte size overflow".into()))?;
    Ok(encoded_tensor_bytes(tensor)?.max(declared))
}

fn validate_tensor_byte_limit(accounted: usize) -> Result<()> {
    if accounted > MAX_ONNX_TENSOR_BYTES {
        return Err(ScanError::Unsupported(
            "ONNX initializer exceeds the per-tensor safety limit".into(),
        ));
    }
    Ok(())
}

fn add_tensor_bytes(budget: &mut ProtoBudget, accounted: usize) -> Result<()> {
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

fn encoded_tensor_bytes(tensor: &tract_onnx::pb::TensorProto) -> Result<usize> {
    let fixed = [
        (tensor.float_data.len(), std::mem::size_of::<f32>()),
        (tensor.int32_data.len(), std::mem::size_of::<i32>()),
        (tensor.int64_data.len(), std::mem::size_of::<i64>()),
        (tensor.double_data.len(), std::mem::size_of::<f64>()),
        (tensor.uint64_data.len(), std::mem::size_of::<u64>()),
    ];
    let typed = fixed
        .into_iter()
        .try_fold(tensor.raw_data.len(), |total, (length, width)| {
            total
                .checked_add(length.saturating_mul(width))
                .ok_or_else(|| ScanError::Invalid("ONNX initializer size overflow".into()))
        })?;
    tensor.string_data.iter().try_fold(typed, |total, value| {
        total
            .checked_add(value.len())
            .ok_or_else(|| ScanError::Invalid("ONNX initializer size overflow".into()))
    })
}

pub(super) fn checked_proto_elements(dims: &[i64]) -> Result<usize> {
    if dims.len() > MAX_ONNX_TENSOR_RANK {
        return Err(rank_error());
    }
    let elements = dims.iter().try_fold(1usize, |total, &dimension| {
        usize::try_from(dimension)
            .map_err(|_| ScanError::Invalid("ONNX tensor has a negative dimension".into()))
            .and_then(|dimension| {
                total
                    .checked_mul(dimension)
                    .ok_or_else(|| ScanError::Invalid("ONNX tensor dimensions overflow".into()))
            })
    })?;
    if elements > MAX_ONNX_TENSOR_ELEMENTS {
        return Err(ScanError::Unsupported(format!(
            "ONNX tensor exceeds the {MAX_ONNX_TENSOR_ELEMENTS}-element safety limit"
        )));
    }
    Ok(elements)
}

fn rank_error() -> ScanError {
    ScanError::Unsupported(format!(
        "ONNX tensor rank exceeds the {MAX_ONNX_TENSOR_RANK}-axis safety limit"
    ))
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
