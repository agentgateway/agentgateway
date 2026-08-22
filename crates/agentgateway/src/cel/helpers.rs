use std::sync::Arc;

use bytes::Bytes;
use cel::Value;
use cel::objects::BytesValue;
use serde::Deserialize;

use super::Expression;

/// Serde `deserialize_with` helper for duration-or-CEL fields: accepts an ergonomic duration literal
/// (e.g. `2s`) by wrapping it as a CEL `duration(...)` call, otherwise parses the value as a CEL
/// expression.
pub fn de_duration_or_expression<'de, D>(deserializer: D) -> Result<Arc<Expression>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let raw = String::deserialize(deserializer)?;
	let expression = if agent_core::durfmt::parse(&raw).is_ok() {
		format!("duration({raw:?})")
	} else {
		raw
	};
	Expression::new_strict(&expression)
		.map(Arc::new)
		.map_err(serde::de::Error::custom)
}

pub fn value_as_byte_or_json(v: Value<'_>) -> anyhow::Result<Bytes> {
	// Materialize Dynamic so nested lookups are converted to concrete values.
	let v = v.always_materialize_owned();
	match &v {
		Value::String(s) => Ok(Bytes::copy_from_slice(s.as_ref().as_bytes())),
		Value::Bytes(BytesValue::Bytes(b)) => Ok(b.clone()),
		Value::Bytes(b) => Ok(Bytes::copy_from_slice(b.as_ref())),
		_ => {
			let js = v.json().map_err(|e| anyhow::anyhow!("{}", e))?;
			let v = serde_json::to_vec(&js)?;
			Ok(Bytes::copy_from_slice(&v))
		},
	}
}
