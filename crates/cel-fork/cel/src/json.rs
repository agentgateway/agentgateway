use base64::prelude::*;
use chrono::Duration;
use thiserror::Error;

use crate::Value;
use crate::duration::format_duration;
use crate::functions::format_timestamp;

#[derive(Debug, Clone, Error)]
#[error("unable to convert value to json: {0:?}")]
pub enum ConvertToJsonError<'a> {
	/// We cannot convert the CEL value to JSON. Some CEL types (like functions) are
	/// not representable in JSON.
	#[error("unable to convert value to json: {0:?}")]
	Value(&'a Value<'a>),

	/// The duration is too large to convert to nanoseconds. Any duration of 2^63
	/// nanoseconds or more will overflow. We'll return the duration type in the
	/// error message.
	#[error("duration too large to convert to nanoseconds: {0:?}")]
	DurationOverflow(&'a Duration),
}

impl<'a> Value<'a> {
	/// Converts a CEL value to a JSON value.
	///
	/// # Example
	/// ```
	/// use cel::{Context, Program};
	///
	/// let program = Program::compile("null").unwrap();
	/// let ctx = Context::default();
	/// let value = program.execute(&ctx).unwrap();
	/// let result = value.json().unwrap();
	///
	/// assert_eq!(result, serde_json::Value::Null);
	/// ```
	pub fn json(&self) -> Result<serde_json::Value, ConvertToJsonError<'_>> {
		Ok(match *self {
			Value::List(ref vec) => serde_json::Value::Array(
				vec
					.iter()
					.map(|v| v.json())
					.collect::<Result<Vec<_>, _>>()?,
			),
			Value::Map(ref map) => {
				let mut obj = serde_json::Map::with_capacity(map.len());
				for (k, v) in map.iter() {
					obj.insert(k.to_string(), v.json().unwrap_or(serde_json::Value::Null));
				}
				serde_json::Value::Object(obj)
			},
			Value::Int(i) => i.into(),
			Value::UInt(u) => u.into(),
			Value::Float(f) => f.into(),
			Value::String(ref s) => s.to_string().into(),
			Value::Bool(b) => b.into(),
			Value::Bytes(ref b) => BASE64_STANDARD.encode(b.as_ref()).to_string().into(),
			Value::Null => serde_json::Value::Null,

			Value::Timestamp(ref dt) => format_timestamp(dt).into(),

			Value::Duration(ref v) => format_duration(v)
				.ok_or(ConvertToJsonError::DurationOverflow(v))?
				.into(),
			Value::Object(ref obj) => obj.json().unwrap_or(serde_json::Value::Null),
			Value::Dynamic(ref d) => {
				let materialized = d.materialize();
				materialized
					.json()
					.map_err(|_| ConvertToJsonError::Value(self))?
			},
			Value::Type(ref t) => t.name().into(),
		})
	}
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use chrono::Duration;
	use serde_json::json;

	use crate::Value as CelValue;
	use crate::common::types;
	use crate::objects::{ListValue, MapValue};

	#[test]
	fn test_cel_value_to_json() {
		let mut tests = vec![
			(json!("hello"), CelValue::String("hello".to_string().into())),
			(json!(42), CelValue::Int(42)),
			(json!(42.0), CelValue::Float(42.0)),
			(json!(true), CelValue::Bool(true)),
			(json!(null), CelValue::Null),
			(json!("int"), CelValue::Type(types::INT_TYPE)),
			(
				json!([true, null]),
				CelValue::List(ListValue::Owned(
					vec![CelValue::Bool(true), CelValue::Null].into(),
				)),
			),
			(
				json!({"hello": "world"}),
				CelValue::Map(MapValue::from(HashMap::from([(
					"hello".to_string(),
					CelValue::String("world".to_string().into()),
				)]))),
			),
		];

		tests.push((json!("1s"), CelValue::Duration(Duration::seconds(1))));
		tests.push((
			json!("61.5s"),
			CelValue::Duration(Duration::seconds(61) + Duration::milliseconds(500)),
		));

		for (expected, value) in tests.iter() {
			assert_eq!(value.json().unwrap(), *expected, "{value:?}={expected:?}");
		}
	}
}
