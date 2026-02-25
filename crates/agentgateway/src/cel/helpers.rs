use bytes::Bytes;
use cel::Value;
use cel::objects::BytesValue;

/// Returns bytes from String or Bytes values. Does not materialize Dynamic (would require
/// returning owned data); callers that need Dynamic support should use value_as_byte_or_json.
pub fn value_as_bytes<'a>(v: &'a Value<'a>) -> Option<&'a [u8]> {
	match v {
		Value::String(b) => Some(b.as_ref().as_bytes()),
		Value::Bytes(b) => Some(b.as_ref()),
		_ => None,
	}
}

pub fn value_as_int(v: &Value) -> Option<i64> {
	// Materialize Dynamic so nested lookups (e.g. jwt.sub) are converted to concrete values.
	let v = v.always_materialize();
	let v = v.as_ref();
	match v {
		Value::Int(b) => Some(*b),
		Value::UInt(b) => Some(i64::try_from(*b).ok()?),
		_ => None,
	}
}

pub fn value_as_string(v: &Value) -> Option<String> {
	// Materialize Dynamic so nested lookups (e.g. jwt.sub) are converted to concrete values.
	let v = v.always_materialize();
	let v = v.as_ref();
	match v {
		Value::String(v) => Some(v.to_string()),
		Value::Bool(v) => Some(v.to_string()),
		Value::Int(v) => Some(v.to_string()),
		Value::UInt(v) => Some(v.to_string()),
		Value::Bytes(v) => {
			use base64::Engine;
			Some(base64::prelude::BASE64_STANDARD.encode(v.as_ref()))
		},
		_ => None,
	}
}

pub fn value_as_header_value(v: &Value) -> Option<http::HeaderValue> {
	// Materialize Dynamic so nested lookups (e.g. jwt.sub) are converted to concrete values.
	let v = v.always_materialize();
	let v = v.as_ref();
	match v {
		Value::String(v) => Some(http::HeaderValue::from_str(v.as_ref()).ok()?),
		Value::Bool(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::Int(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::UInt(v) => Some(http::HeaderValue::from_str(&v.to_string()).ok()?),
		Value::Bytes(v) => {
			use base64::Engine;
			let b = base64::prelude::BASE64_STANDARD.encode(v.as_ref());
			Some(http::HeaderValue::from_str(&b).ok()?)
		},
		_ => None,
	}
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
