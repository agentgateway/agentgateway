use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json::value::RawValue;

use super::{Fields, decode_string, replace_string_values, string_values};

/// A JSON field borrowed from encoded bytes or an already parsed request.
#[derive(Clone, Copy, Debug)]
pub enum JsonRef<'a> {
	Raw(&'a RawValue),
	Parsed(&'a Value),
}

/// JSON borrowed from a request, or owned after an edit.
#[derive(Clone, Debug)]
pub enum Json<'a> {
	Raw(Cow<'a, RawValue>),
	Parsed(Cow<'a, Value>),
}

impl Json<'_> {
	/// Borrows this field without copying its JSON.
	#[must_use]
	pub fn as_ref(&self) -> JsonRef<'_> {
		match self {
			Self::Raw(value) => JsonRef::Raw(value),
			Self::Parsed(value) => JsonRef::Parsed(value),
		}
	}

	/// Keeps the original spelling for raw input; serializes parsed input.
	#[must_use]
	pub fn encoded(&self) -> Cow<'_, str> {
		self.as_ref().encoded()
	}
}

impl<'a> JsonRef<'a> {
	/// Reads a typed field directly from its current representation.
	pub fn read<T: Deserialize<'a>>(self) -> Result<T, serde_json::Error> {
		match self {
			Self::Raw(raw) => serde_json::from_str(raw.get()),
			Self::Parsed(value) => T::deserialize(value),
		}
	}

	/// Keeps the original spelling for raw input; serializes parsed input.
	#[must_use]
	pub fn encoded(self) -> Cow<'a, str> {
		match self {
			Self::Raw(raw) => Cow::Borrowed(raw.get()),
			Self::Parsed(value) => match serde_json::to_string(value) {
				Ok(json) => Cow::Owned(json),
				Err(_) => unreachable!("a JSON value serializes"),
			},
		}
	}

	pub(crate) fn fields(self) -> Result<Fields<'a>, serde_json::Error> {
		match self {
			Self::Raw(raw) => serde_json::from_str(raw.get()),
			Self::Parsed(Value::Object(values)) => Ok(Fields(
				values
					.iter()
					.map(|(name, value)| (Cow::Borrowed(name.as_str()), Self::Parsed(value)))
					.collect(),
			)),
			Self::Parsed(_) => Err(serde::de::Error::custom("expected an object")),
		}
	}

	pub(crate) fn list(self) -> Result<Vec<Self>, serde_json::Error> {
		match self {
			Self::Raw(raw) => serde_json::from_str(raw.get()),
			Self::Parsed(Value::Array(values)) => Ok(values.iter().map(Self::Parsed).collect()),
			Self::Parsed(_) => Err(serde::de::Error::custom("expected a list")),
		}
	}

	pub(crate) fn is_string(self) -> bool {
		match self {
			Self::Raw(raw) => raw.get().starts_with('"'),
			Self::Parsed(value) => value.is_string(),
		}
	}

	pub(crate) fn is_null(self) -> bool {
		match self {
			Self::Raw(raw) => raw.get() == "null",
			Self::Parsed(value) => value.is_null(),
		}
	}

	pub(crate) fn is_array(self) -> bool {
		match self {
			Self::Raw(raw) => raw.get().starts_with('['),
			Self::Parsed(value) => value.is_array(),
		}
	}

	pub(crate) fn is_object(self) -> bool {
		match self {
			Self::Raw(raw) => raw.get().starts_with('{'),
			Self::Parsed(value) => value.is_object(),
		}
	}

	pub(crate) fn text(self) -> Option<Cow<'a, str>> {
		match self {
			Self::Raw(raw) => {
				#[derive(Deserialize)]
				struct Text<'a>(#[serde(borrow)] Cow<'a, str>);
				let Text(text) = serde_json::from_str(raw.get()).ok()?;
				Some(text)
			},
			Self::Parsed(value) => value.as_str().map(Cow::Borrowed),
		}
	}

	pub(crate) fn to_raw(self) -> Box<RawValue> {
		match self {
			Self::Raw(raw) => raw.to_owned(),
			Self::Parsed(value) => match serde_json::value::to_raw_value(value) {
				Ok(raw) => raw,
				Err(_) => unreachable!("a JSON value serializes"),
			},
		}
	}

	pub(crate) fn strings(self) -> JsonStrings<'a> {
		match self {
			Self::Raw(raw) => JsonStrings::Raw {
				json: raw.get(),
				offset: 0,
			},
			Self::Parsed(value) => JsonStrings::Parsed(vec![value]),
		}
	}

	pub(crate) fn replace_strings(
		self,
		mut edit: impl FnMut(usize, Cow<'_, str>) -> Option<String>,
	) -> Option<Json<'static>> {
		match self {
			Self::Raw(raw) => {
				let edited = replace_string_values(raw.get(), edit)?;
				let raw = match RawValue::from_string(edited) {
					Ok(raw) => raw,
					Err(_) => unreachable!("replacing strings keeps JSON valid"),
				};
				Some(raw.into())
			},
			Self::Parsed(value) => {
				let edits: Vec<_> = self
					.strings()
					.enumerate()
					.filter_map(|(at, text)| edit(at, text).map(|text| (at, text)))
					.collect();
				if edits.is_empty() {
					return None;
				}
				let mut value = value.clone();
				replace_values(&mut value, &mut 0, &mut edits.into_iter().peekable());
				Some(value.into())
			},
		}
	}
}

pub enum JsonStrings<'a> {
	Raw { json: &'a str, offset: usize },
	Parsed(Vec<&'a Value>),
}

impl<'a> Iterator for JsonStrings<'a> {
	type Item = Cow<'a, str>;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Raw { json, offset } => {
				let range = string_values(&json[*offset..]).next()?;
				let start = *offset + range.start;
				*offset += range.end;
				Some(decode_string(&json[start..*offset]))
			},
			Self::Parsed(pending) => {
				while let Some(value) = pending.pop() {
					match value {
						Value::String(text) => return Some(Cow::Borrowed(text)),
						Value::Array(values) => pending.extend(values.iter().rev()),
						Value::Object(values) => pending.extend(values.values().rev()),
						_ => {},
					}
				}
				None
			},
		}
	}
}

fn replace_values(
	value: &mut Value,
	at: &mut usize,
	edits: &mut std::iter::Peekable<std::vec::IntoIter<(usize, String)>>,
) {
	match value {
		Value::String(text) => {
			if edits.peek().is_some_and(|(next, _)| next == at) {
				let Some((_, replacement)) = edits.next() else {
					unreachable!("peek found an edit")
				};
				*text = replacement;
			}
			*at += 1;
		},
		Value::Array(values) => {
			for value in values {
				replace_values(value, at, edits);
			}
		},
		Value::Object(values) => {
			for value in values.values_mut() {
				replace_values(value, at, edits);
			}
		},
		_ => {},
	}
}

impl Serialize for JsonRef<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			Self::Raw(raw) => raw.serialize(serializer),
			Self::Parsed(value) => value.serialize(serializer),
		}
	}
}

impl Serialize for Json<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.as_ref().serialize(serializer)
	}
}

impl<'de> Deserialize<'de> for JsonRef<'de> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		<&RawValue>::deserialize(deserializer).map(Self::Raw)
	}
}

impl<'a> From<&'a RawValue> for JsonRef<'a> {
	fn from(value: &'a RawValue) -> Self {
		Self::Raw(value)
	}
}

impl<'a> From<&'a Box<RawValue>> for JsonRef<'a> {
	fn from(value: &'a Box<RawValue>) -> Self {
		Self::Raw(value)
	}
}

impl<'a> From<&Self> for JsonRef<'a> {
	fn from(value: &Self) -> Self {
		*value
	}
}

impl<'a> From<JsonRef<'a>> for Json<'a> {
	fn from(value: JsonRef<'a>) -> Self {
		match value {
			JsonRef::Raw(raw) => Self::Raw(Cow::Borrowed(raw)),
			JsonRef::Parsed(value) => Self::Parsed(Cow::Borrowed(value)),
		}
	}
}

impl<'a> From<Cow<'a, RawValue>> for Json<'a> {
	fn from(value: Cow<'a, RawValue>) -> Self {
		Self::Raw(value)
	}
}

impl From<Box<RawValue>> for Json<'_> {
	fn from(value: Box<RawValue>) -> Self {
		Self::Raw(Cow::Owned(value))
	}
}

impl From<Value> for Json<'_> {
	fn from(value: Value) -> Self {
		Self::Parsed(Cow::Owned(value))
	}
}

impl<'a> From<&'a Json<'_>> for JsonRef<'a> {
	fn from(value: &'a Json<'_>) -> Self {
		value.as_ref()
	}
}
impl<'a> From<&'a Cow<'_, RawValue>> for JsonRef<'a> {
	fn from(value: &'a Cow<'_, RawValue>) -> Self {
		Self::Raw(value)
	}
}
