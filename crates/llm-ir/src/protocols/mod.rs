//! Protocol adapters translate between wire formats and the shared request/event models.
//!
//! Each adapter separates client request parsing (`parse`), provider request rendering
//! (`render`), provider reply decoding (`decoder`), and client reply writing (`writer`).
//! `wire` holds serialization shapes; `parse` also reads a passthrough request's recorded
//! settings without imposing cross-protocol conversion rules. Compatibility is documented
//! on each module.
//!
//! Request parsers try a single-pass read to avoid rescanning nested text.
//! This read rejects lone surrogate escapes and out-of-range numbers.
//! Bodies it cannot read are parsed field by field to report errors.

use std::borrow::Cow;
use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{Deserializer, Error, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use crate::carrier::IdentifierPseudonymizer;
use crate::ir::conversation;
use crate::json::{Fields, JsonRef, Name, is_strict_object};
use crate::prompt::diagnostics::Diagnostics;

pub mod chat_completions;
pub mod gemini;
pub mod messages;
pub(crate) mod reasoning;
pub mod responses;

/// A supported client request and reply format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientProtocol {
	Messages,
	Responses,
	ChatCompletions,
	Gemini,
}

/// Whether a request expects one response body or a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
	Buffered,
	Streaming,
}

impl Delivery {
	#[must_use]
	pub fn is_streaming(self) -> bool {
		self == Self::Streaming
	}
}

/// A client protocol plus endpoint information not carried in its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientEndpoint {
	Messages,
	Responses,
	ChatCompletions,
	/// Gemini chooses streaming in the URL rather than the request body.
	Gemini(Delivery),
}

impl ClientEndpoint {
	#[must_use]
	pub const fn protocol(self) -> ClientProtocol {
		match self {
			Self::Messages => ClientProtocol::Messages,
			Self::Responses => ClientProtocol::Responses,
			Self::ChatCompletions => ClientProtocol::ChatCompletions,
			Self::Gemini(_) => ClientProtocol::Gemini,
		}
	}
}

impl From<ClientProtocol> for ClientEndpoint {
	fn from(protocol: ClientProtocol) -> Self {
		match protocol {
			ClientProtocol::Messages => Self::Messages,
			ClientProtocol::Responses => Self::Responses,
			ClientProtocol::ChatCompletions => Self::ChatCompletions,
			// `generateContent` is the protocol's non-streaming endpoint.
			ClientProtocol::Gemini => Self::Gemini(Delivery::Buffered),
		}
	}
}

/// A supported provider request and reply format, including its capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamProtocol {
	Messages,
	Responses {
		/// The endpoint accepts `safety_identifier`.
		safety_identifier: bool,
		/// The selected model accepts `reasoning.context`.
		reasoning_context: bool,
	},
	ChatCompletions,
	Gemini,
}

/// How one flattened tool name reads to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRoute {
	pub flat: String,
	pub namespace: Option<String>,
	pub name: String,
	pub kind: CallKind,
}

/// The item a client expects back when the model calls this tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
	Function,
	/// Free text, sent upstream as the one `input` field of a function.
	Custom,
	/// The client runs the search, so it wants its own call item back.
	ToolSearch,
}

impl From<&conversation::ToolKind> for CallKind {
	fn from(kind: &conversation::ToolKind) -> Self {
		match kind {
			conversation::ToolKind::Function { .. } => Self::Function,
			conversation::ToolKind::Custom { .. } => Self::Custom,
			conversation::ToolKind::ToolSearch => Self::ToolSearch,
		}
	}
}

/// A complete HTTP reply body in the client's protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
	pub status: u16,
	pub body: Vec<u8>,
}

const NAMESPACE_SEPARATOR: &str = "__";
const MAX_FIELD_NAME: usize = 64;

/// One name for a tool in a namespace, for providers that have none.
pub(crate) fn flat_name(namespace: Option<&str>, name: &str) -> String {
	match namespace.filter(|namespace| *namespace != "functions") {
		Some(namespace) => format!("{namespace}{NAMESPACE_SEPARATOR}{name}"),
		None => name.to_string(),
	}
}

/// Repeated discovery keeps its history, but sends an identical tool declaration once.
pub(crate) fn tool_declarations<'a, 's>(
	tools: &'a [conversation::Tool<'s>],
) -> impl Iterator<Item = &'a conversation::Tool<'s>> {
	tools.iter().enumerate().filter_map(move |(at, tool)| {
		let repeated = tools[..at].iter().any(|earlier| {
			earlier.namespace == tool.namespace
				&& earlier.name == tool.name
				&& earlier.kind == tool.kind
				&& earlier.description == tool.description
				&& same_tool_schema(earlier.schema.as_ref(), tool.schema.as_ref())
		});
		(!repeated).then_some(tool)
	})
}

fn same_tool_schema(left: JsonRef<'_>, right: JsonRef<'_>) -> bool {
	if left.is_object() && right.is_object() {
		let (Ok(left), Ok(right)) = (left.fields(), right.fields()) else {
			return false;
		};
		left.0.len() == right.0.len()
			&& left.0.iter().all(|(name, value)| {
				right
					.0
					.iter()
					.find(|(other, _)| name == other)
					.is_some_and(|(_, other)| same_tool_schema(*value, *other))
			})
	} else if left.is_array() && right.is_array() {
		let (Ok(left), Ok(right)) = (left.list(), right.list()) else {
			return false;
		};
		left.len() == right.len()
			&& left
				.into_iter()
				.zip(right)
				.all(|(left, right)| same_tool_schema(left, right))
	} else if left.is_string() && right.is_string() {
		left
			.text()
			.is_some_and(|text| right.text().as_deref() == Some(text.as_ref()))
	} else {
		// Comparing numbers through Value can round distinct schema constraints to
		// the same float. Keep their original spelling instead.
		left.encoded() == right.encoded()
	}
}

/// Whether a provider's name for a refused field looks like a field name, so a
/// client writer can pass it on.
pub(crate) fn is_field_name(field: &str) -> bool {
	!field.is_empty()
		&& field.len() <= MAX_FIELD_NAME
		&& field
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || b"._-[]".contains(&byte))
}

/// Responses and Chat Completions share this error shape.
pub(crate) fn openai_error_field(body: &[u8]) -> Option<String> {
	#[derive(Deserialize)]
	struct Body<'a> {
		#[serde(borrow)]
		error: Detail<'a>,
	}
	#[derive(Deserialize)]
	struct Detail<'a> {
		#[serde(borrow, default)]
		param: Option<Cow<'a, str>>,
	}
	let parsed: Body<'_> = serde_json::from_slice(body).ok()?;
	parsed.error.param.map(Cow::into_owned)
}

pub(super) enum Read<'a, T> {
	Raw(JsonRef<'a>),
	Done(T),
}

/// Text, or blocks read as `B`.
pub(super) enum ContentRead<'a, B = Fields<'a>> {
	Text(Cow<'a, str>),
	Blocks(Vec<Read<'a, B>>),
}

/// One map's fields, with the value of `nested` read as `T`.
pub(super) fn read_fields<'de, M, T>(
	map: M,
	nested: &str,
) -> Result<(Fields<'de>, Option<T>), M::Error>
where
	M: MapAccess<'de>,
	T: Deserialize<'de>,
{
	read_fields_unless(map, nested, |_| false)
}

/// As [`read_fields`], but fails on a field that `other_spelling` takes for `nested` under
/// another spelling.
pub(super) fn read_fields_unless<'de, M, T>(
	mut map: M,
	nested: &str,
	other_spelling: impl Fn(&str) -> bool,
) -> Result<(Fields<'de>, Option<T>), M::Error>
where
	M: MapAccess<'de>,
	T: Deserialize<'de>,
{
	let (mut fields, mut value) = (Vec::new(), None);
	while let Some(Name(name)) = map.next_key()? {
		if name != nested {
			if other_spelling(&name) {
				return Err(M::Error::custom("nested field in another spelling"));
			}
			fields.push((name, map.next_value::<JsonRef<'de>>()?));
		} else if value.is_none() {
			value = Some(map.next_value::<T>()?);
		} else {
			return Err(M::Error::custom("repeated field"));
		}
	}
	Ok((Fields(fields), value))
}

/// The top-level `model`, read without building the rest of the request, as a
/// typed read of the whole body reads it: escapes are decoded in the key and the
/// name, `null` is the same as missing, and a repeat is refused. Other values are
/// checked only as JSON; the request parser reads them.
pub fn read_model(body: &[u8]) -> Result<Option<Cow<'_, str>>, serde_json::Error> {
	serde_json::from_slice::<TopLevelModel<'_>>(body).map(|read| read.0)
}

struct TopLevelModel<'a>(Option<Cow<'a, str>>);

impl<'de> Deserialize<'de> for TopLevelModel<'de> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		deserializer.deserialize_map(TopLevelModelVisitor)
	}
}

struct TopLevelModelVisitor;

impl<'de> Visitor<'de> for TopLevelModelVisitor {
	type Value = TopLevelModel<'de>;

	fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "a request object")
	}

	fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
		let mut model = None;
		while let Some(IsModel(is_model)) = map.next_key()? {
			if !is_model {
				map.next_value::<IgnoredAny>()?;
			} else if model.is_none() {
				model = Some(map.next_value::<Option<Name<'de>>>()?);
			} else {
				return Err(M::Error::duplicate_field("model"));
			}
		}
		Ok(TopLevelModel(model.flatten().map(|name| name.0)))
	}
}

/// Whether a key, once its escapes are decoded, is `model`.
struct IsModel(bool);

impl<'de> Deserialize<'de> for IsModel {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct KeyVisitor;

		impl Visitor<'_> for KeyVisitor {
			type Value = IsModel;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a field name")
			}

			fn visit_str<E: Error>(self, key: &str) -> Result<Self::Value, E> {
				Ok(IsModel(key == "model"))
			}
		}

		deserializer.deserialize_str(KeyVisitor)
	}
}

/// A top-level `model` value as [`read_model`] reads it.
pub(crate) fn model_name<'a>(
	raw: impl Into<JsonRef<'a>>,
) -> Result<Option<Cow<'a, str>>, serde_json::Error> {
	raw
		.into()
		.read::<Option<Name>>()
		.map(|name| name.map(|name| name.0))
}

impl<'de: 'a, 'a, B: Deserialize<'de>> Deserialize<'de> for ContentRead<'a, B> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct ContentVisitor<B>(PhantomData<B>);

		impl<'de, B: Deserialize<'de>> Visitor<'de> for ContentVisitor<B> {
			type Value = ContentRead<'de, B>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "text or a list of blocks")
			}

			fn visit_borrowed_str<E: Error>(self, text: &'de str) -> Result<Self::Value, E> {
				Ok(ContentRead::Text(Cow::Borrowed(text)))
			}

			fn visit_str<E: Error>(self, text: &str) -> Result<Self::Value, E> {
				Ok(ContentRead::Text(Cow::Owned(text.to_string())))
			}

			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
				let mut blocks = Vec::new();
				while let Some(block) = seq.next_element::<B>()? {
					blocks.push(Read::Done(block));
				}
				Ok(ContentRead::Blocks(blocks))
			}
		}

		deserializer.deserialize_any(ContentVisitor(PhantomData))
	}
}

/// Maximum client identifier length in bytes before pseudonymization.
const MAX_IDENTIFIER: usize = 64;

pub(crate) fn short_identifier(
	pseudonymizer: &dyn IdentifierPseudonymizer,
	user: String,
) -> String {
	if user.len() <= MAX_IDENTIFIER {
		return user;
	}
	let pseudonym = pseudonymizer.pseudonymize(&user);
	assert!(
		pseudonym.len() <= MAX_IDENTIFIER,
		"an identifier pseudonym fits the provider's limit"
	);
	pseudonym
}

/// Wraps a system message that came after the conversation started, for a provider that
/// takes system text only before it, so the message can go in a user turn at its place.
pub(crate) fn system_reminder(text: &str) -> String {
	format!("<system-reminder>\n{text}\n</system-reminder>")
}

/// The provider item a carrier holds, passed on byte for byte, when it is a JSON object.
pub(crate) fn provider_item(item: &[u8]) -> Option<&RawValue> {
	let item = serde_json::from_slice::<&RawValue>(item).ok()?;
	is_strict_object(item).then_some(item)
}

/// Passed on as sent, because which models accept these settings varies. A model
/// that doesn't accept one returns its own error.
pub(crate) fn check_sampling(settings: &conversation::Settings<'_>, diagnostics: &mut Diagnostics) {
	for (path, raw) in [
		("temperature", settings.temperature),
		("top_p", settings.top_p),
	] {
		if let Some(raw) = raw {
			diagnostics.typed::<f64>(path, raw);
		}
	}
}

pub(crate) fn read_image(
	path: &str,
	fields: &conversation::ImageFields<'_>,
	diagnostics: &mut Diagnostics,
) -> Option<conversation::DataImage<'static>> {
	let mut source = None;
	for (name, raw) in &fields.0.0 {
		match name.as_ref() {
			"type" => {},
			"source" => source = Some(*raw),
			// Read with the block, by the request's parser.
			"cache_control" => {},
			other => diagnostics.problem(format!("field `{path}.{other}`")),
		}
	}
	let Some(source) = source else {
		diagnostics.problem(format!("`{path}` has no source"));
		return None;
	};
	let path = format!("{path}.source");
	let source = diagnostics.object(&path, source)?;
	let (mut media_type, mut data) = (None, None);
	for (name, raw) in &source.0 {
		match name.as_ref() {
			"type" => {},
			"media_type" => media_type = diagnostics.typed::<String>(&format!("{path}.media_type"), raw),
			"data" => data = diagnostics.typed::<String>(&format!("{path}.data"), raw),
			other => diagnostics.problem(format!("field `{path}.{other}`")),
		}
	}
	if source.kind().as_deref() != Some("base64") {
		diagnostics.problem(format!("`{path}` must be base64"));
		return None;
	}
	match (media_type, data) {
		(Some(media_type), Some(data)) if is_image(&media_type, &data) => {
			Some(conversation::DataImage {
				media_type,
				data: Cow::Owned(data),
			})
		},
		_ => {
			diagnostics.problem(format!("`{path}` is not a supported image"));
			None
		},
	}
}

/// Any target would refuse a tool it must call but wasn't given. A stage can remove
/// tools after parsing, so this is checked on the way out.
pub(crate) fn check_tool_choice(
	settings: &conversation::Settings<'_>,
	tools: &[conversation::Tool<'_>],
	diagnostics: &mut Diagnostics,
) {
	match &settings.tool_choice {
		Some(conversation::ToolChoice::Required) if tools.is_empty() => {
			diagnostics.problem("`tool_choice` needs a tool to call");
		},
		Some(conversation::ToolChoice::Tool(name))
			if !tools
				.iter()
				.any(|tool| tool.namespace.is_none() && tool.name == *name) =>
		{
			diagnostics.problem(format!("`tool_choice` names no declared tool `{name}`"));
		},
		_ => {},
	}
}

/// A target that declares only plain functions refuses every other kind of tool.
pub(crate) fn refuse_special_tool(tool: &conversation::Tool<'_>, diagnostics: &mut Diagnostics) {
	let plain = tool.kind == conversation::ToolKind::Function { strict: None };
	if !plain || tool.namespace.is_some() {
		diagnostics.problem(format!("`{}` tool kind", tool.path));
	}
}

/// The provider can't enforce a grammar, so the model at least reads it.
pub(crate) fn custom_description(
	tool: &conversation::Tool<'_>,
	grammar: Option<&str>,
	diagnostics: &mut Diagnostics,
) -> String {
	let mut text = tool.description.clone().unwrap_or_default();
	if let Some(grammar) = grammar {
		text.push_str("\n\nInput must match this grammar:\n");
		text.push_str(grammar);
		diagnostics.drop_field("custom tool grammar enforcement");
	}
	text
}

/// Base64 image data with a type both APIs accept. URLs are refused: fetching
/// one, or passing it on, is a separate decision.
pub(crate) fn is_image(media_type: &str, data: &str) -> bool {
	let known = matches!(
		media_type,
		"image/png" | "image/jpeg" | "image/gif" | "image/webp"
	);
	let padded = data.trim_end_matches('=');
	known
		&& !padded.is_empty()
		&& data.len().is_multiple_of(4)
		&& data.len() - padded.len() <= 2
		&& padded
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
}

/// Claude Code's structured output, from `output_config.format`. Only a JSON schema
/// for an object can be sent.
pub(crate) fn output_format_schema<'r>(
	raw: JsonRef<'r>,
	diagnostics: &mut Diagnostics,
) -> Option<JsonRef<'r>> {
	let fields = diagnostics.object("output_config.format", raw)?;
	let mut schema = None;
	for (name, value) in &fields.0 {
		match name.as_ref() {
			"type" => {},
			"schema" => schema = Some(*value),
			other => diagnostics.problem(format!("field `output_config.format.{other}`")),
		}
	}
	match (fields.kind().as_deref(), schema) {
		(Some("json_schema"), Some(schema)) if is_strict_object(schema) => Some(schema),
		(Some("json_schema"), Some(_)) => {
			diagnostics.problem("`output_config.format.schema` must be a valid object");
			None
		},
		_ => {
			diagnostics.problem("`output_config.format`");
			None
		},
	}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
