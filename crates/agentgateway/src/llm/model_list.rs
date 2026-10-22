//! Renders `/v1/models` entries from model catalog metadata.
//!
//! Collection is format-neutral: [`ModelInfo`] carries what we know about a model, and [`render`]
//! alone decides the wire format.

use agent_core::prelude::Strng;
use agent_core::strng;
use serde::Serialize;

use crate::llm::catalog::{Capabilities, Limits, Modalities, Model, Reasoning};

/// What is known about one listed model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelInfo {
	pub id: String,
	pub created: u64,
	pub provider: Option<Strng>,
	pub name: Option<String>,
	pub limits: Limits,
	pub modalities: Modalities,
	pub capabilities: Capabilities,
	pub reasoning: Reasoning,
}

impl ModelInfo {
	pub fn new(id: String, created: u64, provider: Option<Strng>, catalog: Option<&Model>) -> Self {
		let Some(m) = catalog else {
			return Self {
				id,
				created,
				provider,
				..Default::default()
			};
		};
		Self {
			id,
			created: m
				.released
				.as_deref()
				.and_then(release_timestamp)
				.unwrap_or(created),
			provider,
			name: m.name.clone(),
			limits: m.limits.clone(),
			modalities: m.modalities.clone(),
			capabilities: m.capabilities.clone(),
			reasoning: m.reasoning.clone(),
		}
	}

	/// Combines the targets of a virtual model so that what is advertised holds for every target:
	/// the smallest limits, capabilities all targets agree on, and shared modalities and effort levels.
	pub fn combine(id: String, created: u64, targets: &[ModelInfo]) -> Self {
		let Some((first, rest)) = targets.split_first() else {
			return Self {
				id,
				created,
				provider: Some(strng::literal!("virtual")),
				..Default::default()
			};
		};
		// The smallest limit across targets; unknown if any target's limit is unknown.
		let min = |f: fn(&Limits) -> Option<u64>| {
			targets
				.iter()
				.map(|t| f(&t.limits))
				.collect::<Option<Vec<_>>>()
				.and_then(|v| v.into_iter().min())
		};
		// False if any target is false, true if every target is true, otherwise unknown.
		let all = |f: fn(&Capabilities) -> Option<bool>| {
			let values = targets
				.iter()
				.map(|t| f(&t.capabilities))
				.collect::<Vec<_>>();
			if values.contains(&Some(false)) {
				Some(false)
			} else if values.iter().all(|v| *v == Some(true)) {
				Some(true)
			} else {
				None
			}
		};
		// The values every target lists, in the first target's order.
		let shared = |f: fn(&ModelInfo) -> &Vec<String>| {
			f(first)
				.iter()
				.filter(|m| rest.iter().all(|t| f(t).contains(m)))
				.cloned()
				.collect()
		};
		Self {
			id,
			created,
			provider: Some(strng::literal!("virtual")),
			name: None,
			limits: Limits {
				context: min(|l| l.context),
				input: min(|l| l.input.or(l.context)),
				output: min(|l| l.output),
				..Default::default()
			},
			modalities: Modalities {
				input: shared(|t| &t.modalities.input),
				output: shared(|t| &t.modalities.output),
				..Default::default()
			},
			capabilities: Capabilities {
				reasoning: all(|c| c.reasoning),
				structured_output: all(|c| c.structured_output),
				tool_call: all(|c| c.tool_call),
				temperature: all(|c| c.temperature),
				..Default::default()
			},
			reasoning: Reasoning {
				effort: shared(|t| &t.reasoning.effort),
				..Default::default()
			},
		}
	}
}

/// Parses a catalog release date (YYYY-MM-DD or YYYY-MM) as a unix timestamp.
fn release_timestamp(released: &str) -> Option<u64> {
	let date = chrono::NaiveDate::parse_from_str(released, "%Y-%m-%d")
		.or_else(|_| chrono::NaiveDate::parse_from_str(&format!("{released}-01"), "%Y-%m-%d"))
		.ok()?;
	u64::try_from(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp()).ok()
}

/// Renders the model list. Each entry is the OpenAI model object plus the field names commonly
/// read by other clients (Cohere North, OpenRouter, Anthropic, LiteLLM); fields are omitted when
/// unknown.
pub fn render(models: &[ModelInfo]) -> Vec<u8> {
	let list = List {
		object: "list",
		data: models.iter().map(Entry::new).collect(),
	};
	serde_json::to_vec(&list).expect("model list serializes")
}

/// Renders a single model, as returned by `/v1/models/{id}`.
pub fn render_model(model: &ModelInfo) -> Vec<u8> {
	serde_json::to_vec(&Entry::new(model)).expect("model serializes")
}

#[derive(Serialize)]
struct List<'a> {
	object: &'static str,
	data: Vec<Entry<'a>>,
}

#[derive(Serialize)]
struct Entry<'a> {
	id: &'a str,
	object: &'static str,
	created: u64,
	owned_by: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	name: Option<&'a str>,
	#[serde(skip_serializing_if = "Option::is_none")]
	context_window: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	context_length: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	max_input_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	max_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	architecture: Option<Architecture<'a>>,
	#[serde(skip_serializing_if = "EntryCapabilities::is_empty")]
	capabilities: EntryCapabilities,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	supported_parameters: Vec<&'static str>,
}

#[derive(Serialize)]
struct Architecture<'a> {
	input_modalities: Vec<&'a str>,
	output_modalities: Vec<&'a str>,
}

#[derive(Serialize)]
struct EntryCapabilities {
	#[serde(skip_serializing_if = "Option::is_none")]
	reasoning: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	structured_outputs: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	tools: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	temperature: Option<bool>,
}

impl EntryCapabilities {
	fn is_empty(&self) -> bool {
		self.reasoning.is_none()
			&& self.structured_outputs.is_none()
			&& self.tools.is_none()
			&& self.temperature.is_none()
	}
}

impl<'a> Entry<'a> {
	fn new(m: &'a ModelInfo) -> Self {
		fn modalities(list: &[String]) -> Vec<&str> {
			list
				.iter()
				.map(|m| if m == "pdf" { "file" } else { m.as_str() })
				.collect()
		}
		let caps = &m.capabilities;
		let mut params = Vec::new();
		if m.limits.output.is_some() {
			params.push("max_tokens");
		}
		if caps.reasoning == Some(true) {
			params.push("reasoning");
			if !m.reasoning.effort.is_empty() {
				params.push("reasoning_effort");
			}
		}
		if caps.structured_output == Some(true) {
			params.extend(["response_format", "structured_outputs"]);
		}
		if caps.tool_call == Some(true) {
			params.extend(["tool_choice", "tools"]);
		}
		if caps.temperature == Some(true) {
			params.push("temperature");
		}
		params.sort();
		Self {
			id: &m.id,
			object: "model",
			created: m.created,
			owned_by: m.provider.as_deref().unwrap_or("openai"),
			name: m.name.as_deref(),
			context_window: m.limits.context,
			context_length: m.limits.context,
			max_input_tokens: m.limits.input.or(m.limits.context),
			max_tokens: m.limits.output,
			architecture: (!m.modalities.input.is_empty() || !m.modalities.output.is_empty()).then(
				|| Architecture {
					input_modalities: modalities(&m.modalities.input),
					output_modalities: modalities(&m.modalities.output),
				},
			),
			capabilities: EntryCapabilities {
				reasoning: caps.reasoning,
				structured_outputs: caps.structured_output,
				tools: caps.tool_call,
				temperature: caps.temperature,
			},
			supported_parameters: params,
		}
	}
}
