//! Reads a request the provider gets unconverted, because it serves the client's own protocol.
//!
//! Nothing here is rendered, so a field the conversion pipeline would refuse is no reason to
//! refuse the request: the provider decides. [`RequestFields`] reads the body's top level once,
//! for its model and then for the settings the caller records. [`ParsedRequest`] reads the model
//! and settings of a body the caller already parsed,
//! without scanning its bytes again. [`leading_model`] reads only the model, when it comes
//! before the conversation, so choosing where a request goes needn't scan the conversation.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU64;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use serde_json::value::RawValue;

use super::RequestRefusal;
use crate::ir::conversation::{Effort, Settings};
use crate::json::{Fields, JsonRef, Name};
use crate::protocols::{ClientEndpoint, chat_completions, gemini, messages, responses};

/// A request's top-level fields, read once: for its model, then for a passthrough's settings.
/// The conversation is skipped, which is quicker than keeping it.
pub struct RequestFields<'a> {
	settings: Fields<'a>,
}

impl<'a> RequestFields<'a> {
	/// Refused only when the body isn't a JSON object.
	pub fn read(body: &'a [u8]) -> Result<Self, RequestRefusal> {
		let SettingsFields(settings) =
			serde_json::from_slice(body).map_err(|error| not_an_object(&error))?;
		Ok(Self { settings })
	}

	/// The top-level model, read as [`read_model`](crate::read_model) reads it.
	pub fn model(&self) -> Result<Option<Cow<'a, str>>, RequestRefusal> {
		model(
			self
				.settings
				.0
				.iter()
				.filter(|(name, _)| name == "model")
				.map(|(_, raw)| *raw),
		)
	}

	/// The settings a passthrough attempt records: the output limit, sampling, effort, and
	/// whether the reply streams. Refused only when a setting repeats, or when whether it
	/// streams can't be read.
	pub fn read_passthrough(&self, client: ClientEndpoint) -> Result<Settings<'a>, RequestRefusal> {
		read(&self.settings, client)
	}
}

/// A parsed request borrowed from its caller. Duplicate keys must be checked by the
/// initial parser, since a `Value` keeps one value per key.
pub struct ParsedRequest<'a>(Fields<'a>);

impl<'a> ParsedRequest<'a> {
	/// Refused when the value isn't an object.
	pub fn new(body: &'a Value) -> Result<Self, RequestRefusal> {
		JsonRef::Parsed(body)
			.fields()
			.map(Self)
			.map_err(|error| not_an_object(&error))
	}

	/// The top-level model, read as [`RequestFields::model`] reads it.
	pub fn model(&self) -> Result<Option<Cow<'a, str>>, RequestRefusal> {
		model(
			self
				.0
				.0
				.iter()
				.filter(|(name, _)| name == "model")
				.map(|(_, value)| *value),
		)
	}

	/// The settings [`RequestFields::read_passthrough`] reads.
	pub fn read_passthrough(&self, client: ClientEndpoint) -> Result<Settings<'a>, RequestRefusal> {
		read(&self.0, client)
	}
}

/// A body's top-level model, as far as [`leading_model`] read.
#[derive(Debug, PartialEq)]
pub enum LeadingModel<'a> {
	/// No conversation field came before it. `None` when the body has no model.
	Read(Option<Cow<'a, str>>),
	/// A conversation field came first, so finding the model means scanning the conversation.
	AfterConversation,
}

/// Reads the top-level model without scanning the conversation, when the model comes first.
/// Nothing after the model is read, so a repeated `model` isn't refused here: every reader of
/// the whole body refuses it. Refused when the body isn't a JSON object as far as the model, or
/// the model isn't a name.
pub fn leading_model(body: &[u8]) -> Result<LeadingModel<'_>, RequestRefusal> {
	match first(body, Until::Conversation).map_err(|error| not_an_object(&error))? {
		Some(Leading::Model(raw)) => model(std::iter::once(raw.into())).map(LeadingModel::Read),
		Some(Leading::Conversation) => Ok(LeadingModel::AfterConversation),
		None => Ok(LeadingModel::Read(None)),
	}
}

/// The body with its top-level model replaced and every other byte kept. Only the first model
/// is replaced, so the caller must already have refused a repeated one. `None` when the body
/// has no model, or can't be read as far as it.
#[must_use]
pub fn replace_model(body: &[u8], model: &str) -> Option<Vec<u8>> {
	let Ok(Some(Leading::Model(raw))) = first(body, Until::Model) else {
		return None;
	};
	let raw = raw.get().as_bytes();
	assert!(
		body.as_ptr_range().contains(&raw.as_ptr()),
		"a value read from the body is borrowed from it"
	);
	let start = raw.as_ptr() as usize - body.as_ptr() as usize;
	let mut replaced = Vec::with_capacity(body.len() + model.len());
	replaced.extend_from_slice(&body[..start]);
	match serde_json::to_writer(&mut replaced, model) {
		Ok(()) => {},
		Err(_) => unreachable!("a string writes to a Vec"),
	}
	replaced.extend_from_slice(&body[start + raw.len()..]);
	Some(replaced)
}

/// What comes first at the top level: the model or the conversation.
enum Leading<'a> {
	Model(&'a RawValue),
	Conversation,
}

/// Where reading the top level stops when it finds no model.
#[derive(Clone, Copy, PartialEq)]
enum Until {
	Conversation,
	Model,
}

/// `None` when the object ends first.
fn first(body: &[u8], until: Until) -> Result<Option<Leading<'_>>, serde_json::Error> {
	let mut found = None;
	let mut deserializer = serde_json::Deserializer::from_slice(body);
	match (&mut deserializer).deserialize_map(LeadingVisitor(&mut found, until)) {
		Ok(()) => deserializer.end().map(|()| None),
		// The visitor stops reading with an error once it found what it looked for.
		Err(error) => found.map(Some).ok_or(error),
	}
}

struct LeadingVisitor<'f, 'a>(&'f mut Option<Leading<'a>>, Until);

impl<'de> Visitor<'de> for LeadingVisitor<'_, 'de> {
	type Value = ();

	fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "a JSON object")
	}

	fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
		while let Some(Name(name)) = map.next_key()? {
			if name == "model" {
				*self.0 = Some(Leading::Model(map.next_value()?));
				return Err(serde::de::Error::custom("read as far as the model"));
			}
			if self.1 == Until::Conversation && CONVERSATION.contains(&name.as_ref()) {
				*self.0 = Some(Leading::Conversation);
				return Err(serde::de::Error::custom("read as far as the conversation"));
			}
			map.next_value::<IgnoredAny>()?;
		}
		Ok(())
	}
}

/// A body's top-level fields but the conversation's.
struct SettingsFields<'a>(Fields<'a>);

impl<'de: 'a, 'a> Deserialize<'de> for SettingsFields<'a> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct SettingsVisitor;

		impl<'de> Visitor<'de> for SettingsVisitor {
			type Value = SettingsFields<'de>;

			fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
				write!(f, "a JSON object")
			}

			fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
				let mut fields = Vec::new();
				while let Some(Name(name)) = map.next_key()? {
					// Kept as `RawValue`, a field's text is checked once more for UTF-8.
					if CONVERSATION.contains(&name.as_ref()) {
						map.next_value::<IgnoredAny>()?;
					} else {
						fields.push((name, map.next_value::<JsonRef<'de>>()?));
					}
				}
				Ok(SettingsFields(Fields(fields)))
			}
		}

		deserializer.deserialize_map(SettingsVisitor)
	}
}

/// Fields that hold the conversation, which reading settings never looks at. Skipping them only
/// saves time: one missing here is kept and ignored.
const CONVERSATION: &[&str] = &[
	"messages",
	"input",
	"contents",
	"system",
	"instructions",
	"systemInstruction",
	"system_instruction",
	"tools",
];

fn not_an_object(error: &dyn fmt::Display) -> RequestRefusal {
	RequestRefusal {
		problems: vec![format!("the request isn't a JSON object: {error}")],
	}
}

/// The model from each top-level `model` field: there may be only one.
fn model<'a>(
	mut fields: impl Iterator<Item = JsonRef<'a>>,
) -> Result<Option<Cow<'a, str>>, RequestRefusal> {
	let refused = |problem: &dyn fmt::Display| RequestRefusal {
		problems: vec![format!("`model` can't be read: {problem}")],
	};
	let Some(raw) = fields.next() else {
		return Ok(None);
	};
	if fields.next().is_some() {
		return Err(refused(&"it is repeated"));
	}
	crate::protocols::model_name(raw).map_err(|error| refused(&error))
}

fn read<'a>(fields: &Fields<'a>, client: ClientEndpoint) -> Result<Settings<'a>, RequestRefusal> {
	let mut names: Vec<&str> = fields.0.iter().map(|(name, _)| name.as_ref()).collect();
	names.sort_unstable();
	if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
		return Err(RequestRefusal {
			problems: vec![format!("the request repeats `{}`", pair[0])],
		});
	}
	let mut reading = Reading::default();
	match client {
		ClientEndpoint::Messages => messages::parse::read(fields, &mut reading),
		ClientEndpoint::Responses => responses::parse::read(fields, &mut reading),
		ClientEndpoint::ChatCompletions => chat_completions::parse::read(fields, &mut reading),
		ClientEndpoint::Gemini(delivery) => {
			gemini::parse::read(fields, &mut reading);
			reading.settings.stream = Some(delivery.is_streaming());
		},
	}
	if reading.problems.is_empty() {
		Ok(reading.settings)
	} else {
		Err(RequestRefusal {
			problems: reading.problems,
		})
	}
}

/// What a protocol's reader finds.
#[derive(Default)]
pub(crate) struct Reading<'a> {
	settings: Settings<'a>,
	problems: Vec<String>,
}

impl<'a> Reading<'a> {
	pub(crate) const fn settings(&mut self) -> &mut Settings<'a> {
		&mut self.settings
	}

	pub(crate) fn stream(&mut self, path: &str, raw: JsonRef<'_>) {
		match raw.read::<Option<bool>>() {
			Ok(stream) => self.settings.stream = stream,
			Err(_) => self.problems.push(format!("`{path}` must be a boolean")),
		}
	}

	/// Only the provider reads a malformed limit, so it isn't recorded.
	pub(crate) fn limit(raw: JsonRef<'_>) -> Option<NonZeroU64> {
		raw.read().ok()
	}

	/// A word for effort, as each protocol spells it.
	pub(crate) fn effort(&mut self, word: &str) {
		self.settings.effort = Effort::read(word);
	}
}

/// The value of a field, if the object has it.
pub(crate) fn field<'a>(fields: &Fields<'a>, name: &str) -> Option<JsonRef<'a>> {
	fields
		.0
		.iter()
		.find(|(field, _)| field == name)
		.map(|(_, raw)| *raw)
}

/// A string field, read as a plain value such as an id or a type.
pub(crate) fn word(fields: &Fields<'_>, name: &str) -> Option<String> {
	field(fields, name).and_then(|raw| raw.read().ok())
}

#[cfg(test)]
#[path = "passthrough_tests.rs"]
mod tests;
