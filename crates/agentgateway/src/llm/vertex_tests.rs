use super::Provider;
use crate::llm::RouteType;
use crate::llm::types::messages;
use agent_core::strng;

#[test]
fn test_get_path_for_model_explicit_routes() {
	let provider = Provider {
		model: None,
		region: Some(strng::literal!("us-central1")),
		project_id: strng::literal!("test-project"),
	};

	// Test Messages route with an explicitly Anthropic prefixed model
	let path = provider.get_path_for_model(
		RouteType::Messages,
		Some("anthropic/my-custom-model"),
		false,
	);
	assert_eq!(
		path,
		"/v1/projects/test-project/locations/us-central1/publishers/anthropic/models/my-custom-model:rawPredict"
	);

	// Test Messages route with a non-Anthropic model (e.g. gemini) -> Should use Generic Endpoint
	let path = provider.get_path_for_model(RouteType::Messages, Some("gemini-pro"), false);
	assert_eq!(
		path,
		"/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
	);
}

#[test]
fn test_get_path_for_model_completions_route() {
	let provider = Provider {
		model: None,
		region: Some(strng::literal!("us-central1")),
		project_id: strng::literal!("test-project"),
	};

	// Test Completions route with non-standard model (should fallback to generic Vertex URL)
	let path = provider.get_path_for_model(RouteType::Completions, Some("my-custom-model"), false);
	assert_eq!(
		path,
		"/v1/projects/test-project/locations/us-central1/endpoints/openapi/chat/completions"
	);

	// Test Completions route with Claude model (should detect and use Anthropic URL)
	let path = provider.get_path_for_model(RouteType::Completions, Some("claude-3-5-sonnet"), false);
	assert_eq!(
		path,
		"/v1/projects/test-project/locations/us-central1/publishers/anthropic/models/claude-3-5-sonnet:rawPredict"
	);
}

#[tokio::test]
async fn test_prepare_anthropic_request_body() {
	let provider = Provider {
		model: Some(strng::literal!("claude-3-5-sonnet@20240620")),
		region: Some(strng::literal!("us-central1")),
		project_id: strng::literal!("test-project"),
	};

	let messages_req = messages::Request {
		model: Some("claude-3-5-sonnet@20240620".to_string()),
		messages: vec![messages::RequestMessage {
			role: "user".to_string(),
			content: Some(messages::RequestContent::Text("Hello".to_string())),
			rest: Default::default(),
		}],
		max_tokens: Some(1024),
		stream: Some(true),
		temperature: None,
		top_p: None,
		system: None,
		rest: Default::default(),
	};

	let body = serde_json::to_vec(&messages_req).unwrap();

	let prepared_body = provider.prepare_anthropic_request_body(body).unwrap();

	let prepared_json: serde_json::Value = serde_json::from_slice(&prepared_body).unwrap();

	insta::with_settings!({



		snapshot_path => "tests",



		prepend_module_to_snapshot => false,



	}, {



		insta::assert_json_snapshot!("vertex-request_anthropic_prepare", prepared_json);



	});
}
