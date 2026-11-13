//! Borrowed JSON parsing, validation and serialization for protocol conversion.
//!
//! String values in valid JSON, found without parsing the rest, so editing one
//! leaves keys, numbers and their order as they were.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::value::RawValue;

pub mod input;
mod value;
pub use value::{Json, JsonRef};

pub fn text_of<'a>(value: impl Into<JsonRef<'a>>) -> Option<Cow<'a, str>> {
	value.into().text()
}

/// `RawValue` checks structure but not every value: a lone surrogate escape or a
/// number out of range gets through. Text passed on unread must be read strictly
/// here, or the provider sees something this gateway accepted and can't parse.
pub fn is_strict_object<'a>(value: impl Into<JsonRef<'a>>) -> bool {
	match value.into() {
		JsonRef::Raw(raw) => raw.get().starts_with('{') && is_strict(raw.get()),
		JsonRef::Parsed(value) => value.is_object(),
	}
}

pub fn is_strict(text: &str) -> bool {
	serde_json::from_str::<Strict>(text).is_ok()
}

/// Refuses repeated keys, because two readers can pick different values.
struct Strict;

impl<'de> Deserialize<'de> for Strict {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_any(StrictVisitor)
	}
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
	type Value = Strict;

	fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "JSON")
	}

	fn visit_bool<E>(self, _: bool) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_i64<E>(self, _: i64) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_u64<E>(self, _: u64) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_f64<E>(self, _: f64) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_str<E>(self, _: &str) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_unit<E>(self) -> Result<Strict, E> {
		Ok(Strict)
	}

	fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
		while seq.next_element::<Strict>()?.is_some() {}
		Ok(Strict)
	}

	fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Strict, M::Error> {
		let mut names = HashSet::new();
		while let Some(name) = map.next_key::<String>()? {
			if !names.insert(name) {
				return Err(de::Error::custom("repeated key"));
			}
			map.next_value::<Strict>()?;
		}
		Ok(Strict)
	}
}

/// An object's fields in order, repeats included. A name is borrowed from the request
/// unless it has an escape.
pub struct Fields<'a>(pub(crate) Vec<(Cow<'a, str>, JsonRef<'a>)>);

impl Serialize for Fields<'_> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		let mut map = serializer.serialize_map(Some(self.0.len()))?;
		for (name, value) in &self.0 {
			map.serialize_entry(name, value)?;
		}
		map.end()
	}
}

impl Fields<'_> {
	pub(crate) fn kind(&self) -> Option<String> {
		let (_, raw) = self.0.iter().find(|(name, _)| name == "type")?;
		raw.read().ok()
	}
}

impl<'de: 'a, 'a> Deserialize<'de> for Fields<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct FieldsVisitor;

		impl<'de> Visitor<'de> for FieldsVisitor {
			type Value = Fields<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a JSON object")
			}

			fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
				let mut fields = Vec::new();
				while let Some(Name(name)) = map.next_key()? {
					fields.push((name, map.next_value::<JsonRef<'de>>()?));
				}
				Ok(Fields(fields))
			}
		}

		deserializer.deserialize_map(FieldsVisitor)
	}
}

/// A field name, borrowed unless it has an escape.
#[derive(serde::Deserialize)]
pub struct Name<'a>(#[serde(borrow)] pub(crate) Cow<'a, str>);

/// Where each string value is, quotes included. A string followed by `:` is an
/// object key, and outside strings no other `"` can appear.
pub fn string_values(json: &str) -> impl Iterator<Item = Range<usize>> + '_ {
	let bytes = json.as_bytes();
	let mut at = 0;
	std::iter::from_fn(move || {
		while let Some(found) = memchr::memchr(b'"', &bytes[at..]) {
			let start = at + found;
			let mut end = start + 1;
			while bytes[end] != b'"' {
				end += if bytes[end] == b'\\' { 2 } else { 1 };
			}
			end += 1;
			at = end;
			let next = bytes[end..].iter().find(|byte| !byte.is_ascii_whitespace());
			if next != Some(&b':') {
				return Some(start..end);
			}
		}
		None
	})
}

/// A checked JSON string literal's text, quotes excluded. Borrowed unless it has escapes.
pub fn decode_string(literal: &str) -> Cow<'_, str> {
	// Checked JSON with no backslash has no escapes, so its text is what's between the
	// quotes. Otherwise it is decoded once: serde's borrowed `&str` would decode the whole
	// string before refusing it.
	if !literal.contains('\\') {
		return Cow::Borrowed(&literal[1..literal.len() - 1]);
	}
	#[derive(serde::Deserialize)]
	struct Text<'a>(#[serde(borrow)] Cow<'a, str>);
	match serde_json::from_str::<Text<'_>>(literal) {
		Ok(Text(text)) => text,
		Err(error) => unreachable!("a checked JSON string decodes: {error}"),
	}
}

/// Calls `edit` with each string value's place among them and its text, and writes
/// back, escaped, what it returns. `None` when it replaced nothing.
pub fn replace_string_values(
	json: &str,
	mut edit: impl FnMut(usize, Cow<'_, str>) -> Option<String>,
) -> Option<String> {
	let mut edited: Option<String> = None;
	let mut copied = 0;
	for (string, range) in string_values(json).enumerate() {
		let Some(replacement) = edit(string, decode_string(&json[range.clone()])) else {
			continue;
		};
		let out = edited.get_or_insert_with(|| String::with_capacity(json.len()));
		out.push_str(&json[copied..range.start]);
		push_string(out, &replacement);
		copied = range.end;
	}
	let mut edited = edited?;
	edited.push_str(&json[copied..]);
	Some(edited)
}

pub fn string_value(text: &str) -> Result<Box<RawValue>, serde_json::Error> {
	serde_json::value::to_raw_value(text)
}

pub fn push_string(out: &mut String, text: &str) {
	let encoded = serde_json::to_string(text)
		.unwrap_or_else(|error| unreachable!("a string serializes: {error}"));
	out.push_str(&encoded);
}

#[cfg(test)]
mod tests;
