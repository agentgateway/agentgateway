//! JSON Schema to the OpenAPI subset Gemini takes, for tool parameters and reply
//! formats. `$ref`s are copied in place, `allOf` is merged, and keywords Gemini
//! doesn't know are dropped. A keyword that changes what's valid and can't be
//! rewritten is refused.

use std::borrow::Cow;
use std::fmt;

use serde_json::value::RawValue;

use crate::json::{Fields, Json, JsonRef, push_string, text_of};
use crate::prompt::diagnostics::Diagnostics;

/// Schema keywords Gemini accepts whose value goes through unchanged. Others are
/// dropped, as validators ignore unknown keywords.
const SCHEMA_KEYWORDS: [&str; 16] = [
	"description",
	"title",
	"nullable",
	"required",
	"default",
	"minLength",
	"maxLength",
	"pattern",
	"minimum",
	"maximum",
	"minItems",
	"maxItems",
	"minProperties",
	"maxProperties",
	"example",
	"propertyOrdering",
];

/// Schema keywords that change what's valid and can't be rewritten here yet.
const SCHEMA_REFUSED: [&str; 2] = ["not", "if"];

/// `$ref`s are copied in place, and each can name others, so a small schema could
/// otherwise grow without limit. This bounds what copying adds to one request.
const MAX_INLINED_SCHEMA_BYTES: usize = 1 << 22;

#[derive(Default)]
pub struct SchemaRefs {
	targets: Vec<(String, Box<RawValue>)>,
	/// A `$ref` already being copied is recursive, which Gemini can't express.
	copying: Vec<String>,
	inlined_bytes: usize,
}

/// `schema_refs` lives as long as the request, so the bound on copied bytes covers
/// all of its schemas. Paths are written out only in problems, so they're passed
/// unformatted.
pub struct SchemaWriter<'r> {
	pub(crate) diagnostics: &'r mut Diagnostics,
	pub(crate) schema_refs: &'r mut SchemaRefs,
}

impl SchemaWriter<'_> {
	pub(crate) fn root_schema(&mut self, path: &str, raw: JsonRef<'_>) -> Option<String> {
		let fields = self.diagnostics.object(path, raw)?;
		let mut targets = Vec::new();
		for (name, value) in &fields.0 {
			if matches!(name.as_ref(), "$defs" | "definitions") {
				let group = self
					.diagnostics
					.object(format_args!("{path}.{name}"), value)?;
				for (target, schema) in group.0 {
					targets.push((format!("#/{name}/{target}"), schema.to_raw()));
				}
			}
		}
		self.schema_refs.targets = targets;
		self.object_schema(&path, fields)
	}

	fn schema(&mut self, path: &dyn fmt::Display, raw: JsonRef<'_>) -> Option<String> {
		let fields = self.diagnostics.object(path, raw)?;
		self.object_schema(path, fields)
	}

	fn object_schema(&mut self, path: &dyn fmt::Display, fields: Fields<'_>) -> Option<String> {
		let fields = self.flatten_all_of(path, fields)?;
		let fields = self.simple_any_of(path, fields)?;
		if let Some((_, reference)) = fields.iter().find(|(name, _)| name == "$ref") {
			let reference = reference.clone();
			return self.referenced_schema(path, reference.as_ref(), &fields);
		}
		let mut kept = Vec::with_capacity(fields.len());
		let (mut kind, mut has_enum, mut has_any_of, mut has_items) = (None, false, false, false);
		for (name, value) in &fields {
			let value = value.as_ref();
			match name.as_ref() {
				"type" => {
					// Most types are one name. Failing to read that as a list builds an error message.
					let list = value.is_array();
					if let Some(types) = list.then(|| value.read::<Vec<String>>().ok()).flatten() {
						let (null, types): (Vec<_>, Vec<_>) =
							types.into_iter().partition(|name| name == "null");
						if !null.is_empty() && !has_keyword(&fields, "nullable") {
							kept.push(("nullable", Cow::Borrowed("true")));
						}
						match types.as_slice() {
							[] => {},
							[only] => kind = Some(only.clone()),
							many => {
								has_any_of = true;
								kept.push(("anyOf", any_of_types(many).into()));
							},
						}
					} else {
						kind = self
							.diagnostics
							.typed::<String>(format_args!("{path}.type"), value);
					}
					continue;
				},
				"const" => {
					if text_of(value).is_some() && !has_keyword(&fields, "enum") {
						kind.get_or_insert_with(|| "string".to_string());
						kept.push(("enum", format!("[{}]", value.encoded()).into()));
					} else {
						self.diagnostics.drop_field("tool schema keyword");
					}
					continue;
				},
				"$defs" | "definitions" => continue,
				"enum" => has_enum = true,
				"properties" => {
					kept.push((
						name.as_ref(),
						self
							.schema_map(&format_args!("{path}.properties"), value)?
							.into(),
					));
					continue;
				},
				"items" => {
					has_items = true;
					kept.push((
						name.as_ref(),
						self.schema(&format_args!("{path}.items"), value)?.into(),
					));
					continue;
				},
				"anyOf" => {
					has_any_of = true;
					kept.push((
						name.as_ref(),
						self
							.schema_list(&format_args!("{path}.anyOf"), value)?
							.into(),
					));
					continue;
				},
				"format" => {
					let format = self
						.diagnostics
						.typed::<String>(format_args!("{path}.format"), value);
					if matches!(format.as_deref(), Some("enum" | "date-time")) {
						kept.push((name.as_ref(), value.encoded()));
					}
					continue;
				},
				known if SCHEMA_KEYWORDS.contains(&known) => {},
				refused if SCHEMA_REFUSED.contains(&refused) => {
					self
						.diagnostics
						.problem(format!("`{path}` keyword `{refused}`"));
					continue;
				},
				_ => {
					self.diagnostics.drop_field("tool schema keyword");
					continue;
				},
			}
			if name != "enum" {
				kept.push((name.as_ref(), value.encoded()));
			}
		}
		if has_enum {
			let values = fields
				.iter()
				.find(|(name, _)| name == "enum")
				.map(|(_, value)| value);
			match values {
				Some(values)
					if matches!(kind.as_deref(), None | Some("string"))
						&& values.as_ref().read::<Vec<String>>().is_ok() =>
				{
					kind = Some("string".to_string());
					kept.push(("enum", values.encoded()));
				},
				// Gemini takes `enum` only on strings.
				_ => self.diagnostics.drop_field("tool schema keyword"),
			}
		}
		if kind.is_none() && !has_any_of {
			kind = Some("object".to_string());
		}
		let mut out = String::from("{");
		if let Some(kind) = &kind {
			out.push_str("\"type\":");
			push_string(&mut out, kind);
			out.push(',');
		}
		// Gemini requires `items` on arrays.
		if kind.as_deref() == Some("array") && !has_items {
			out.push_str("\"items\":{\"type\":\"object\"},");
		}
		for (name, value) in kept {
			push_string(&mut out, name);
			out.push(':');
			out.push_str(&value);
			out.push(',');
		}
		if out.ends_with(',') {
			out.pop();
		}
		out.push('}');
		Some(out)
	}

	/// `oneOf` becomes `anyOf`, the closest Gemini has, and a `null` member becomes
	/// `nullable`. A lone member left over takes the place of the list, so a `$ref`
	/// in `Optional[...]` is seen. The node's own keywords win over the member's.
	fn simple_any_of<'a>(
		&mut self,
		path: &dyn fmt::Display,
		mut fields: SchemaFields<'a>,
	) -> Option<SchemaFields<'a>> {
		if has_keyword(&fields, "oneOf") {
			if has_keyword(&fields, "anyOf") {
				self
					.diagnostics
					.problem(format!("`{path}` keyword `oneOf` beside `anyOf`"));
				return None;
			}
			self.diagnostics.drop_field("tool schema oneOf");
		}
		let Some(at) = fields
			.iter()
			.position(|(name, _)| matches!(name.as_ref(), "anyOf" | "oneOf"))
		else {
			return Some(fields);
		};
		fields[at].0 = Cow::Borrowed("anyOf");
		let list_path = format!("{path}.anyOf");
		let members = self.diagnostics.list(&list_path, &fields[at].1)?;
		let mut kept = Vec::with_capacity(members.len());
		for (index, member) in members.iter().enumerate() {
			let object = self
				.diagnostics
				.object(format_args!("{list_path}[{index}]"), member)?;
			if object.kind().as_deref() != Some("null") {
				kept.push(*member);
			}
		}
		let had_null = kept.len() < members.len();
		if kept.len() == members.len() && kept.len() != 1 {
			return Some(fields);
		}
		let mut merged = Vec::new();
		match kept.as_slice() {
			[] => {},
			[only] => {
				for (name, value) in self.diagnostics.object(&list_path, only)?.0 {
					merged.push((Cow::Owned(name.into_owned()), Json::from(value.to_raw())));
				}
			},
			many => {
				let list = serde_json::value::to_raw_value(many)
					.unwrap_or_else(|error| unreachable!("schema members serialize: {error}"));
				merged.push((Cow::Borrowed("anyOf"), Json::from(list)));
			},
		}
		fields.remove(at);
		for (name, value) in merged {
			if !has_keyword(&fields, &name) {
				fields.push((name, value));
			}
		}
		if had_null && !has_keyword(&fields, "nullable") {
			fields.push((
				Cow::Borrowed("nullable"),
				Json::from(serde_json::Value::Bool(true)),
			));
		}
		Some(fields)
	}

	fn referenced_schema(
		&mut self,
		path: &dyn fmt::Display,
		reference: JsonRef<'_>,
		fields: &SchemaFields<'_>,
	) -> Option<String> {
		let reference = self
			.diagnostics
			.typed::<String>(format_args!("{path}.$ref"), reference)?;
		if self.schema_refs.copying.contains(&reference) {
			self
				.diagnostics
				.problem(format!("`{path}` `$ref` `{reference}` is recursive"));
			return None;
		}
		let target = self
			.schema_refs
			.targets
			.iter()
			.find(|(name, _)| *name == reference)
			.map(|(_, target)| target.clone());
		let Some(target) = target else {
			self
				.diagnostics
				.problem(format!("`{path}` `$ref` `{reference}` isn't defined here"));
			return None;
		};
		self.schema_refs.inlined_bytes += target.get().len();
		if self.schema_refs.inlined_bytes > MAX_INLINED_SCHEMA_BYTES {
			self
				.diagnostics
				.problem(format!("`{path}` `$ref`s copy too much"));
			return None;
		}
		let copied = self.diagnostics.object(path, &target)?;
		let mut merged: Vec<_> = fields
			.iter()
			.filter(|(name, _)| name != "$ref")
			.map(|(name, value)| (Cow::Borrowed(name.as_ref()), value.as_ref()))
			.collect();
		for (name, value) in copied.0 {
			if !has_keyword(fields, &name) {
				merged.push((name, value));
			}
		}
		self.schema_refs.copying.push(reference);
		let schema = self.object_schema(path, Fields(merged));
		self.schema_refs.copying.pop();
		schema
	}

	/// Gemini has no `allOf`. The first keyword value wins except that `properties`
	/// and `required` are merged. Later constraints may be lost; flattening is
	/// recorded rather than refused. The merged schema is still checked for refused keywords.
	fn flatten_all_of<'a>(
		&mut self,
		path: &dyn fmt::Display,
		fields: Fields<'a>,
	) -> Option<SchemaFields<'a>> {
		let mut out = Vec::with_capacity(fields.0.len());
		let mut members = None;
		for (name, value) in fields.0 {
			if name == "allOf" {
				members = Some(value);
			} else {
				out.push((name, Json::from(value)));
			}
		}
		let Some(members) = members else {
			return Some(out);
		};
		self.diagnostics.drop_field("tool schema allOf");
		let path = format!("{path}.allOf");
		for (at, member) in self
			.diagnostics
			.list(&path, members)?
			.into_iter()
			.enumerate()
		{
			let path = format!("{path}[{at}]");
			let member = self.diagnostics.object(&path, member)?;
			for (name, value) in self.flatten_all_of(&path, member)? {
				self.merge_keyword(&path, &mut out, name, value)?;
			}
		}
		Some(out)
	}

	fn merge_keyword<'a>(
		&mut self,
		path: &dyn fmt::Display,
		out: &mut SchemaFields<'a>,
		name: Cow<'a, str>,
		value: Json<'a>,
	) -> Option<()> {
		let Some(at) = out.iter().position(|(existing, _)| *existing == name) else {
			out.push((name, value));
			return Some(());
		};
		let joined = match name.as_ref() {
			"properties" => self.join_properties(path, out[at].1.as_ref(), value.as_ref())?,
			"required" => self.join_required(path, out[at].1.as_ref(), value.as_ref())?,
			_ => return Some(()),
		};
		out[at].1 = Json::from(joined);
		Some(())
	}

	fn join_properties(
		&mut self,
		path: &dyn fmt::Display,
		first: JsonRef<'_>,
		next: JsonRef<'_>,
	) -> Option<Box<RawValue>> {
		let mut fields = self
			.diagnostics
			.object(format_args!("{path}.properties"), first)?;
		let next = self
			.diagnostics
			.object(format_args!("{path}.properties"), next)?;
		for (name, value) in next.0 {
			if !fields.0.iter().any(|(existing, _)| *existing == name) {
				fields.0.push((name, value));
			}
		}
		Some(
			serde_json::value::to_raw_value(&fields)
				.unwrap_or_else(|error| unreachable!("schema properties serialize: {error}")),
		)
	}

	fn join_required(
		&mut self,
		path: &dyn fmt::Display,
		first: JsonRef<'_>,
		next: JsonRef<'_>,
	) -> Option<Box<RawValue>> {
		let mut names: Vec<String> = self
			.diagnostics
			.typed(format_args!("{path}.required"), first)?;
		for name in self
			.diagnostics
			.typed::<Vec<String>>(format_args!("{path}.required"), next)?
		{
			if !names.contains(&name) {
				names.push(name);
			}
		}
		Some(
			serde_json::value::to_raw_value(&names)
				.unwrap_or_else(|error| unreachable!("required names serialize: {error}")),
		)
	}

	/// `properties`: its keys are names, not keywords.
	fn schema_map(&mut self, path: &dyn fmt::Display, raw: JsonRef<'_>) -> Option<String> {
		let fields = self.diagnostics.object(path, raw)?;
		let mut out = String::from("{");
		for (at, (name, value)) in fields.0.iter().enumerate() {
			if at > 0 {
				out.push(',');
			}
			push_string(&mut out, name);
			out.push(':');
			out.push_str(&self.schema(&format_args!("{path}.{name}"), *value)?);
		}
		out.push('}');
		Some(out)
	}

	fn schema_list(&mut self, path: &dyn fmt::Display, raw: JsonRef<'_>) -> Option<String> {
		let items = self.diagnostics.list(path, raw)?;
		let mut out = String::from("[");
		for (at, item) in items.iter().enumerate() {
			if at > 0 {
				out.push(',');
			}
			out.push_str(&self.schema(&format_args!("{path}[{at}]"), *item)?);
		}
		out.push(']');
		Some(out)
	}
}

type SchemaFields<'a> = Vec<(Cow<'a, str>, Json<'a>)>;

fn has_keyword(fields: &SchemaFields<'_>, keyword: &str) -> bool {
	fields.iter().any(|(name, _)| name == keyword)
}

fn any_of_types(types: &[String]) -> String {
	let mut out = String::from("[");
	for (at, name) in types.iter().enumerate() {
		if at > 0 {
			out.push(',');
		}
		out.push_str("{\"type\":");
		push_string(&mut out, name);
		out.push('}');
	}
	out.push(']');
	out
}
