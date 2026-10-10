use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// TypeSafe /v1/systemone request. See https://docs.typesafe.ai/api.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
	pub model: String,
	/// A string, object, or array.
	pub state: serde_json::Value,
	pub questions: IndexMap<String, Question>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
	Noul {
		/// A string, object, or array.
		instructions: serde_json::Value,
		#[serde(skip_serializing_if = "Option::is_none")]
		criteria: Option<NoulCriteria>,
	},
	Choice {
		/// A string, object, or array.
		instructions: serde_json::Value,
		/// Option name to optional description.
		criteria: IndexMap<String, Option<String>>,
	},
	Score {
		/// A string, object, or array.
		instructions: serde_json::Value,
		/// Level descriptions, lowest first. A string, object, or array.
		criteria: Vec<serde_json::Value>,
	},
}

#[derive(Debug, Clone, Serialize)]
pub struct NoulCriteria {
	#[serde(rename = "true")]
	pub true_: String,
	#[serde(rename = "false")]
	pub false_: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
	pub model: String,
	pub answers: IndexMap<String, Answer>,
	pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Deserialize)]
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
		score: f64,
		probabilities: IndexMap<String, f64>,
		confidence: f64,
	},
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
	pub input_tokens: u32,
	pub output_tokens: u32,
}
