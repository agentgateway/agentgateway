use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
pub struct PredictRequest {
	pub instances: Vec<Instance>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub parameters: Option<Parameters>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Instance {
	pub content: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub task_type: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Parameters {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub auto_truncate: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_dimensionality: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PredictResponse {
	pub predictions: Vec<Prediction>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Prediction {
	pub embeddings: EmbeddingsResult,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EmbeddingsResult {
	pub values: Vec<f32>,
	pub statistics: Option<Statistics>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Statistics {
	pub token_count: u64,
}

// ---- Vertex Discovery Engine Ranking API ----

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankRequest {
	pub model: String,
	pub query: String,
	pub records: Vec<RankRecord>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_n: Option<u32>,
	pub ignore_record_details_in_response: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RankRecord {
	pub id: String,
	pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RankResponse {
	#[serde(default)]
	pub records: Vec<RankResponseRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RankResponseRecord {
	pub id: String,
	#[serde(default)]
	pub score: Option<f64>,
}
