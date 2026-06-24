use crate::llm::types;

// Request/response translation fidelity is covered by the golden snapshot tests in
// `llm/tests.rs` (`requests/rerank/*` and `response/*/rerank*.json`); this module covers the
// error and host-resolution paths.

fn bedrock_provider(model: &str, region: &str) -> crate::llm::bedrock::Provider {
	crate::llm::bedrock::Provider {
		model: Some(agent_core::strng::new(model)),
		region: agent_core::strng::new(region),
		guardrail_identifier: None,
		guardrail_version: None,
		source_credentials_cache: Default::default(),
		assume_role_cache: Default::default(),
	}
}

#[test]
fn test_bedrock_rerank_request_passes_through_full_arn() {
	let arn = "arn:aws:bedrock:us-east-1::foundation-model/cohere.rerank-v3-5:0";
	let req: types::rerank::Request =
		serde_json::from_str(r#"{"query":"q","documents":["a"]}"#).unwrap();
	let provider = bedrock_provider(arn, "us-east-1");
	let out = crate::llm::conversion::bedrock::from_rerank::translate(&req, &provider).unwrap();
	let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
	assert_eq!(
		v["rerankingConfiguration"]["bedrockRerankingConfiguration"]["modelConfiguration"]["modelArn"],
		arn
	);
}

#[test]
fn test_bedrock_rerank_uses_agent_runtime_host_and_rerank_path() {
	let provider = bedrock_provider("cohere.rerank-v3-5:0", "us-west-2");
	assert_eq!(
		provider.get_host(crate::llm::RouteType::Rerank).as_str(),
		"bedrock-agent-runtime.us-west-2.amazonaws.com"
	);
	assert_eq!(
		provider
			.get_path_for_route(crate::llm::RouteType::Rerank, false, "cohere.rerank-v3-5:0")
			.as_str(),
		"/rerank"
	);
	assert_eq!(
		provider
			.get_host(crate::llm::RouteType::Embeddings)
			.as_str(),
		"bedrock-runtime.us-west-2.amazonaws.com"
	);
}

/// Asserts the final TCP/TLS target `default_connector_target` resolves per route, which keeps the
/// rerank connection off `bedrock-runtime`.
#[test]
fn test_bedrock_connection_target_is_route_aware() {
	use crate::llm::{AIProvider, RouteType};
	use crate::types::agent::Target;

	let provider = AIProvider::Bedrock(bedrock_provider("cohere.rerank-v3-5:0", "us-west-2"));

	let resolved_host = |route: RouteType| -> String {
		match provider
			.default_connector_target(route)
			.expect("bedrock has a default target")
		{
			Target::Hostname(h, port) => {
				assert_eq!(port, 443);
				h.to_string()
			},
			other => panic!("expected hostname target, got {other:?}"),
		}
	};

	assert_eq!(
		resolved_host(RouteType::Rerank),
		"bedrock-agent-runtime.us-west-2.amazonaws.com"
	);
	assert_eq!(
		resolved_host(RouteType::Embeddings),
		"bedrock-runtime.us-west-2.amazonaws.com"
	);
	assert_eq!(
		resolved_host(RouteType::Completions),
		"bedrock-runtime.us-west-2.amazonaws.com"
	);
}

#[test]
fn test_bedrock_rerank_error_translation() {
	let error_body = bytes::Bytes::from(
		serde_json::to_vec(&serde_json::json!({"message": "model not found"})).unwrap(),
	);
	let translated =
		crate::llm::conversion::bedrock::from_rerank::translate_error(&error_body).unwrap();
	let error_resp: serde_json::Value = serde_json::from_slice(&translated).unwrap();
	assert_eq!(error_resp["error"]["type"], "invalid_request_error");
	assert_eq!(error_resp["error"]["message"], "model not found");
}

#[test]
fn test_bedrock_rerank_empty_documents_errors() {
	let req: types::rerank::Request =
		serde_json::from_str(r#"{"query":"q","documents":[]}"#).unwrap();
	let provider = bedrock_provider("cohere.rerank-v3-5:0", "us-east-1");
	assert!(crate::llm::conversion::bedrock::from_rerank::translate(&req, &provider).is_err());
}

// ---- Vertex Discovery Engine Ranking ----

fn vertex_provider(project: &str, region: &str) -> crate::llm::vertex::Provider {
	crate::llm::vertex::Provider {
		model: None,
		region: Some(agent_core::strng::new(region)),
		project_id: agent_core::strng::new(project),
	}
}

#[test]
fn test_vertex_rerank_response_rejects_non_numeric_id() {
	let vertex_resp = r#"{"records":[{"id":"abc","score":0.5}]}"#;
	let result =
		crate::llm::conversion::vertex::from_rerank::translate_response(vertex_resp.as_bytes());
	assert!(result.is_err());
}

/// Mirrors `test_bedrock_connection_target_is_route_aware` for Vertex: asserts the route-aware target
/// keeps the rerank connection off the `aiplatform` host.
#[test]
fn test_vertex_connection_target_is_route_aware() {
	use crate::llm::{AIProvider, RouteType};
	use crate::types::agent::Target;

	let provider = AIProvider::Vertex(vertex_provider("proj", "us-central1"));

	let resolved_host = |route: RouteType| -> String {
		match provider
			.default_connector_target(route)
			.expect("vertex has a default target")
		{
			Target::Hostname(h, port) => {
				assert_eq!(port, 443);
				h.to_string()
			},
			other => panic!("expected hostname target, got {other:?}"),
		}
	};

	assert_eq!(
		resolved_host(RouteType::Rerank),
		"discoveryengine.googleapis.com"
	);
	assert_eq!(
		resolved_host(RouteType::Completions),
		"us-central1-aiplatform.googleapis.com"
	);
	assert_eq!(
		resolved_host(RouteType::Embeddings),
		"us-central1-aiplatform.googleapis.com"
	);
}

#[test]
fn test_vertex_rerank_uses_discovery_engine_host_and_ranking_path() {
	use crate::llm::RouteType;

	let provider = vertex_provider("proj", "global");
	assert_eq!(
		crate::llm::vertex::DISCOVERY_ENGINE_HOST.as_str(),
		"discoveryengine.googleapis.com"
	);
	let path = provider.get_path_for_model(RouteType::Rerank, None, false);
	assert!(
		path
			.as_str()
			.ends_with("rankingConfigs/default_ranking_config:rank"),
		"path was {path}"
	);
	assert!(path.as_str().contains("/projects/proj/"), "path was {path}");
}

#[test]
fn test_vertex_rerank_error_translation() {
	let error_body = bytes::Bytes::from(
		serde_json::to_vec(&serde_json::json!({
			"error": {
				"code": 403,
				"message": "Discovery Engine API has not been used in project before or it is disabled",
				"status": "PERMISSION_DENIED"
			}
		}))
		.unwrap(),
	);
	let translated =
		crate::llm::conversion::vertex::from_rerank::translate_error(&error_body).unwrap();
	let error_resp: serde_json::Value = serde_json::from_slice(&translated).unwrap();
	assert_eq!(error_resp["error"]["type"], "authentication_error");
	assert_eq!(
		error_resp["error"]["message"],
		"Discovery Engine API has not been used in project before or it is disabled"
	);
}

#[test]
fn test_vertex_rerank_empty_documents_errors() {
	let req: types::rerank::Request =
		serde_json::from_str(r#"{"query":"q","documents":[]}"#).unwrap();
	let provider = vertex_provider("proj", "global");
	assert!(crate::llm::conversion::vertex::from_rerank::translate(&req, &provider).is_err());
}
