use std::collections::HashMap;
use std::sync::Arc;

use prost_wkt_types::{Struct, Value};
use tracing::warn;

use crate::cel::{self, Executor};
use crate::http::envoy_proto_common;
use crate::proxy::ProxyError;

/// Reserved internal metadata_context namespace whose key/value pairs are copied
/// into outbound gRPC initial metadata when an ext_proc stream is opened.
pub(crate) const EXTPROC_GRPC_INITIAL_METADATA_NAMESPACE: &str =
	"agentgateway.dev.grpc_initial_metadata";

pub(crate) type MetadataContext = HashMap<String, HashMap<String, Arc<cel::Expression>>>;

pub(crate) fn eval_expression(exec: &Executor, v: &cel::Expression) -> Result<Value, ProxyError> {
	let res = exec.eval(v).map_err(|e| ProxyError::Processing(e.into()))?;
	let js = res
		.json()
		.map_err(|_| ProxyError::Processing(cel::Error::JsonConvert.into()))?;
	envoy_proto_common::json_to_prost_value(js)
}

pub(crate) fn eval_to_struct(
	exec: &Executor<'_>,
	expressions: &HashMap<String, Arc<cel::Expression>>,
) -> Result<Struct, ProxyError> {
	Ok(Struct {
		fields: expressions
			.iter()
			.filter_map(|(key, expr)| match eval_expression(exec, expr) {
				Ok(result) => Some((key.clone(), result)),
				Err(error) => {
					warn!(%key, %error, "failed to evaluate metadata_context CEL expression");
					None
				},
			})
			.collect(),
	})
}

pub(crate) fn build_processing_metadata_context(
	exec: &Executor<'_>,
	metadata_context: Option<&MetadataContext>,
) -> Option<HashMap<String, Struct>> {
	metadata_context.map(|meta| {
		meta
			// The reserved namespace is transported as gRPC initial metadata
			// instead of regular ext_proc metadata_context.
			.iter()
			.filter(|(namespace, _)| namespace.as_str() != EXTPROC_GRPC_INITIAL_METADATA_NAMESPACE)
			.filter_map(|(namespace, expressions)| {
				eval_to_struct(exec, expressions)
					.map(|value| (namespace.clone(), value))
					.ok()
			})
			.collect()
	})
}

pub(crate) fn build_grpc_initial_metadata(
	exec: &Executor<'_>,
	metadata_context: Option<&MetadataContext>,
) -> tonic::metadata::MetadataMap {
	let mut metadata = tonic::metadata::MetadataMap::new();
	let Some(expressions) =
		metadata_context.and_then(|ctx| ctx.get(EXTPROC_GRPC_INITIAL_METADATA_NAMESPACE))
	else {
		return metadata;
	};

	// CEL values in the reserved namespace are copied into the outbound gRPC
	// stream-open metadata, skipping entries that cannot be represented there.
	for (key, expr) in expressions {
		let value = match eval_expression(exec, expr) {
			Ok(value) => value,
			Err(error) => {
				warn!(
					%key,
					%error,
					"failed to evaluate gRPC initial metadata CEL expression"
				);
				continue;
			},
		};
		let Some(string_value) = prost_value_to_metadata_string(&value) else {
			continue;
		};
		let metadata_key = match tonic::metadata::MetadataKey::from_bytes(key.as_bytes()) {
			Ok(metadata_key) => metadata_key,
			Err(error) => {
				warn!(%key, %error, "failed to convert gRPC initial metadata key");
				continue;
			},
		};
		let metadata_value = match tonic::metadata::MetadataValue::try_from(string_value.as_str()) {
			Ok(metadata_value) => metadata_value,
			Err(error) => {
				warn!(
					%key,
					value = %string_value,
					%error,
					"failed to convert gRPC initial metadata value"
				);
				continue;
			},
		};
		metadata.insert(metadata_key, metadata_value);
	}

	metadata
}

fn prost_value_to_metadata_string(value: &Value) -> Option<String> {
	use prost_wkt_types::value::Kind;

	match value.kind.as_ref()? {
		Kind::StringValue(s) => Some(s.clone()),
		Kind::NumberValue(n) => Some(n.to_string()),
		Kind::BoolValue(b) => Some(b.to_string()),
		Kind::NullValue(_) => None,
		Kind::StructValue(_) | Kind::ListValue(_) => None,
	}
}
