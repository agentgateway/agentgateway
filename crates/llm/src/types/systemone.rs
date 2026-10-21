use agent_core::prelude::Strng;
use agent_core::strng;
use serde::{Deserialize, Serialize};

use crate::types::{ContentScope, RequestType};
use crate::{AIError, InputFormat, LLMRequest, LLMRequestParams, SimpleChatCompletionMessage};

/// TypeSafe /v1/systemone request. See https://docs.typesafe.ai/api.
#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<typed::Usage>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

impl RequestType for Request {
	fn input_format() -> InputFormat {
		InputFormat::SystemOne
	}
	fn body_is_json(&self) -> bool {
		true
	}
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn to_value(&self) -> serde_json::Result<serde_json::Value> {
		serde_json::to_value(self)
	}

	fn prepend_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn append_prompts(&mut self, _prompts: Vec<SimpleChatCompletionMessage>) {}

	fn to_llm_request(&self, provider: Strng, _tokenize: bool) -> Result<LLMRequest, AIError> {
		Ok(LLMRequest {
			input_tokens: None,
			input_format: InputFormat::SystemOne,
			cache_convention: crate::CacheTokenConvention::pending(),
			request_model: strng::new(self.model.as_deref().unwrap_or_default()),
			provider,
			streaming: false,
			params: LLMRequestParams::default(),
			prompt: Default::default(),
			provider_state: None,
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		unimplemented!("get_messages is used for prompt guard; prompt guard is disabled for systemone.")
	}

	fn set_messages(&mut self, _messages: Vec<SimpleChatCompletionMessage>) {
		unimplemented!("set_messages is used for prompt guard; prompt guard is disabled for systemone.")
	}

	fn visit_text_mut(&mut self, _f: &mut dyn FnMut(ContentScope, &mut String)) {
		unimplemented!(
			"visit_text_mut is used for prompt guard; prompt guard is disabled for systemone."
		)
	}
}

impl crate::types::ResponseType for Response {
	fn to_llm_response(&self, _log_content: crate::LogContentFields) -> crate::LLMResponse {
		let usage = self.usage.as_ref();
		crate::LLMResponse {
			input_tokens: usage.map(|u| u.input_tokens as u64),
			output_tokens: usage.map(|u| u.output_tokens as u64),
			total_tokens: usage.map(|u| (u.input_tokens + u.output_tokens) as u64),
			provider_model: self.model.as_deref().map(strng::new),
			..Default::default()
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
		vec![]
	}

	fn set_webhook_choices(
		&mut self,
		_resp: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(self)
	}

	fn visit_text_mut(&mut self, _f: &mut dyn FnMut(crate::types::ResponseText, &mut String)) {}
}

pub mod typed {
	use indexmap::IndexMap;
	use serde::{Deserialize, Serialize};

	#[derive(Debug, Clone, Serialize, Deserialize)]
	pub struct Request {
		pub model: String,
		/// A string, object, or array.
		pub state: serde_json::Value,
		pub questions: IndexMap<String, Question>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(tag = "type", rename_all = "lowercase")]
	pub enum Question {
		Noul {
			/// A string, object, or array.
			instructions: serde_json::Value,
			#[serde(default, skip_serializing_if = "Option::is_none")]
			criteria: Option<NoulCriteria>,
		},
		Choice {
			/// A string, object, or array.
			instructions: serde_json::Value,
			/// Option name to optional description. A string, object, or array.
			criteria: IndexMap<String, Option<serde_json::Value>>,
		},
		Score {
			/// A string, object, or array.
			instructions: serde_json::Value,
			/// Level descriptions, lowest first. A string, object, or array.
			criteria: Vec<serde_json::Value>,
		},
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	pub struct NoulCriteria {
		/// A string, object, or array.
		#[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
		pub true_: Option<serde_json::Value>,
		/// A string, object, or array.
		#[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
		pub false_: Option<serde_json::Value>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	pub struct Response {
		pub model: String,
		pub answers: IndexMap<String, Answer>,
		#[serde(skip_serializing_if = "Option::is_none")]
		pub usage: Option<Usage>,
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	#[serde(tag = "type", rename_all = "lowercase")]
	pub enum Answer {
		Noul {
			/// Probability of `true`, 0-1.
			noul: f64,
		},
		Choice {
			choice: String,
			probabilities: IndexMap<String, f64>,
			confidence: f64,
		},
		Score {
			score: serde_json::Number,
			/// Level to its criteria. A string, object, or array.
			#[serde(default, skip_serializing_if = "Option::is_none")]
			legend: Option<IndexMap<String, serde_json::Value>>,
			probabilities: IndexMap<String, f64>,
			confidence: f64,
		},
	}

	#[derive(Debug, Clone, Serialize, Deserialize)]
	pub struct Usage {
		pub input_tokens: u32,
		pub output_tokens: u32,
	}
}
