use bytes::Bytes;
use cel::Value;
use cel::objects::BytesValue;

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
