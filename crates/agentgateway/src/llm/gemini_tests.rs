use std::collections::HashMap;

use agent_core::strng;
use http_body_util::BodyExt;
use serde_json::Value;

use crate::http::Body;
use crate::llm::policy::Policy;
use crate::llm::{
	AIError, AIProvider, CacheTokenConvention, InputFormat, LLMRequest, ProviderState, RequestResult,
	RouteType, anthropic, bedrock, gemini, openai, vertex,
};
use crate::types::agent::Target;

// ── fixtures ────────────────────────────────────────────────────────────────

fn vertex_provider(region: Option<&str>, model: Option<&str>) -> AIProvider {
	AIProvider::Vertex(vertex::Provider {
		project_id: strng::new("test-project"),
		model: model.map(strng::new),
		region: region.map(strng::new),
	})
}

fn gemini_provider(model: Option<&str>) -> AIProvider {
	AIProvider::Gemini(gemini::Provider {
		model: model.map(strng::new),
	})
}

/// What `process_gemini_request` hands to `setup_request` for a chat route: the upstream route
/// type is `GenerateContent` (the provider format the native translation renders into) and the
/// native render is signalled by `ProviderState::VertexGemini`.
fn native_chat_request(model: &str, streaming: bool) -> LLMRequest {
	LLMRequest {
		input_tokens: None,
		input_format: InputFormat::Gemini,
		cache_convention: CacheTokenConvention::pending(),
		request_model: model.into(),
		provider: Default::default(),
		streaming,
		params: Default::default(),
		prompt: None,
		provider_state: Some(ProviderState::VertexGemini),
	}
}

/// countTokens has no chat translation, so it never carries `provider_state`; its client-facing
/// route type is what reaches `setup_request`.
fn count_tokens_request(model: &str) -> LLMRequest {
	LLMRequest {
		input_format: InputFormat::GeminiCountTokens,
		provider_state: None,
		..native_chat_request(model, false)
	}
}

fn backend_info(host: &str) -> crate::http::auth::BackendInfo {
	use crate::test_helpers::proxymock::setup_proxy_test;
	use crate::types::agent::BackendTarget;
	crate::http::auth::BackendInfo {
		target: BackendTarget::Invalid,
		call_target: Target::from((host, 443)),
		inputs: setup_proxy_test("{}").unwrap().pi,
	}
}

fn gemini_request(uri: &str, body: &str) -> crate::http::Request {
	::http::Request::builder()
		.uri(uri)
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(body.as_bytes().to_vec()))
		.unwrap()
}

fn generate_content_body(uri: &str) -> crate::http::Request {
	gemini_request(
		uri,
		r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
	)
}

// ── set_required_fields ─────────────────────────────────────────────────────

#[test]
fn set_required_fields_leaves_oauth_bearer_tokens_untouched() {
	// OAuth access tokens (GCP backend-auth, workload identity, ...) stay in
	// `Authorization: Bearer`; only API keys are relocated (see the test below).
	for provider in [
		gemini_provider(None),
		vertex_provider(Some("us-central1"), None),
	] {
		for route_type in [
			RouteType::GenerateContent,
			RouteType::GeminiCountTokens,
			RouteType::Completions,
		] {
			let mut req = crate::http::tests_common::request(
				"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
				::http::Method::POST,
				&[
					("authorization", "Bearer ya29.token"),
					("x-goog-user-project", "p"),
				],
			);
			let before = req.headers().clone();

			provider
				.set_required_fields(
					&mut req,
					route_type,
					Some(&native_chat_request("gemini-2.5-flash", false)),
				)
				.unwrap();

			assert_eq!(req.headers(), &before, "{provider:?} / {route_type:?}");
			assert!(!req.headers().contains_key("anthropic-version"));
		}
	}
}

#[test]
fn set_required_fields_moves_api_keys_to_x_goog_api_key_on_native_routes() {
	// The native endpoints authenticate API keys via `x-goog-api-key`; a key left in
	// `Authorization: Bearer` (the backend-auth default location) would be rejected as an
	// invalid OAuth token.
	let provider = gemini_provider(None);
	for (route_type, llm_request) in [
		(
			RouteType::GenerateContent,
			native_chat_request("gemini-2.5-flash", false),
		),
		(
			RouteType::GeminiCountTokens,
			count_tokens_request("gemini-2.5-flash"),
		),
	] {
		let mut req = crate::http::tests_common::request(
			"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
			::http::Method::POST,
			&[("authorization", "Bearer AIzaTestKey123")],
		);

		provider
			.set_required_fields(&mut req, route_type, Some(&llm_request))
			.unwrap();

		assert!(
			!req.headers().contains_key(::http::header::AUTHORIZATION),
			"{route_type:?}: the API key must not stay in Authorization"
		);
		assert_eq!(
			req.headers().get("x-goog-api-key").unwrap(),
			"AIzaTestKey123",
			"{route_type:?}"
		);
	}
}

#[test]
fn set_required_fields_keeps_api_keys_on_the_compat_shim_and_explicit_locations() {
	// The OpenAI-compat shim accepts `Authorization: Bearer <api key>`, so a non-native
	// route keeps the client's header.
	let provider = gemini_provider(None);
	let shim_request = LLMRequest {
		input_format: InputFormat::Completions,
		provider_state: None,
		..native_chat_request("gemini-2.5-flash", false)
	};
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1/chat/completions",
		::http::Method::POST,
		&[("authorization", "Bearer AIzaTestKey123")],
	);
	provider
		.set_required_fields(&mut req, RouteType::Completions, Some(&shim_request))
		.unwrap();
	assert_eq!(
		req.headers().get(::http::header::AUTHORIZATION).unwrap(),
		"Bearer AIzaTestKey123"
	);
	assert!(!req.headers().contains_key("x-goog-api-key"));

	// An explicitly configured Authorization location must never be rewritten.
	let mut req = crate::http::tests_common::request(
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		::http::Method::POST,
		&[("authorization", "Bearer AIzaTestKey123")],
	);
	req
		.extensions_mut()
		.insert(crate::http::auth::AppliedBackendAuthLocation { explicit: true });
	provider
		.set_required_fields(
			&mut req,
			RouteType::GenerateContent,
			Some(&native_chat_request("gemini-2.5-flash", false)),
		)
		.unwrap();
	assert_eq!(
		req.headers().get(::http::header::AUTHORIZATION).unwrap(),
		"Bearer AIzaTestKey123"
	);
	assert!(!req.headers().contains_key("x-goog-api-key"));
}

// ── upstream path building ──────────────────────────────────────────────────

fn setup(
	provider: &AIProvider,
	uri: &str,
	route_type: RouteType,
	llm_request: &LLMRequest,
) -> crate::http::Request {
	let mut req = crate::http::tests_common::request(uri, ::http::Method::POST, &[]);
	provider
		.setup_request(&mut req, route_type, Some(llm_request), None, None, false)
		.expect("setup_request should succeed");
	req
}

#[rstest::rstest]
#[case::global(
	None,
	false,
	"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-flash:generateContent",
	None,
	"aiplatform.googleapis.com"
)]
#[case::global_streaming(
	None,
	true,
	"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent",
	Some("alt=sse"),
	"aiplatform.googleapis.com"
)]
#[case::regional(
	Some("us-central1"),
	false,
	"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent",
	None,
	"us-central1-aiplatform.googleapis.com"
)]
#[case::regional_streaming(
	Some("us-central1"),
	true,
	"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent",
	Some("alt=sse"),
	"us-central1-aiplatform.googleapis.com"
)]
fn vertex_builds_the_native_publisher_path(
	#[case] region: Option<&str>,
	#[case] streaming: bool,
	#[case] expected_path: &str,
	#[case] expected_query: Option<&str>,
	#[case] expected_authority: &str,
) {
	let provider = vertex_provider(region, None);
	let client_path = if streaming {
		"https://example.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
	} else {
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent"
	};
	let req = setup(
		&provider,
		client_path,
		RouteType::GenerateContent,
		&native_chat_request("gemini-2.5-flash", streaming),
	);

	assert_eq!(req.uri().path(), expected_path);
	assert_eq!(req.uri().query(), expected_query);
	assert_eq!(
		req.uri().authority().map(|a| a.as_str()),
		Some(expected_authority)
	);
}

#[test]
fn vertex_count_tokens_uses_the_publisher_path() {
	let provider = vertex_provider(Some("us-central1"), None);
	let req = setup(
		&provider,
		"https://example.com/v1beta/models/gemini-2.5-flash:countTokens",
		RouteType::GeminiCountTokens,
		&count_tokens_request("gemini-2.5-flash"),
	);

	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:countTokens"
	);
	assert_eq!(req.uri().query(), None);
	assert_eq!(
		req.uri().authority().map(|a| a.as_str()),
		Some("us-central1-aiplatform.googleapis.com")
	);
}

#[test]
fn vertex_native_path_normalizes_resource_style_model_names() {
	// Vertex quickstarts and SDK configs write the model as a resource name; the publisher path
	// supplies its own prefix, so only the bare id may reach it.
	let provider = vertex_provider(None, None);
	for model in [
		"models/gemini-2.5-flash",
		"google/gemini-2.5-flash",
		"publishers/google/models/gemini-2.5-flash",
	] {
		let req = setup(
			&provider,
			"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
			RouteType::GenerateContent,
			&native_chat_request(model, false),
		);
		assert_eq!(
			req.uri().path(),
			"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-flash:generateContent",
			"{model}"
		);
	}
}

#[test]
fn gemini_api_native_paths_cover_all_three_methods() {
	let provider = gemini_provider(None);
	let cases = [
		(
			RouteType::GenerateContent,
			native_chat_request("gemini-2.5-flash", false),
			"/v1beta/models/gemini-2.5-flash:generateContent",
			None,
		),
		(
			RouteType::GenerateContent,
			native_chat_request("gemini-2.5-flash", true),
			"/v1beta/models/gemini-2.5-flash:streamGenerateContent",
			Some("alt=sse"),
		),
		(
			RouteType::GeminiCountTokens,
			count_tokens_request("gemini-2.5-flash"),
			"/v1beta/models/gemini-2.5-flash:countTokens",
			None,
		),
	];
	for (route_type, llm_request, expected_path, expected_query) in cases {
		let req = setup(
			&provider,
			"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
			route_type,
			&llm_request,
		);
		assert_eq!(req.uri().path(), expected_path);
		assert_eq!(req.uri().query(), expected_query);
		assert_eq!(
			req.uri().authority().map(|a| a.as_str()),
			Some(gemini::DEFAULT_HOST_STR)
		);
	}
}

// ── end-to-end: model resolution rewrites the upstream path ─────────────────

async fn process_and_setup(
	provider: &AIProvider,
	policy: Option<&Policy>,
	uri: &str,
	host: &str,
) -> (LLMRequest, crate::http::Request) {
	let RequestResult::Success {
		request: mut req,
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_gemini_request(
			&backend_info(host),
			policy,
			generate_content_body(uri),
			false,
			&mut None,
		)
		.await
		.expect("generateContent request should process")
	else {
		panic!("expected a forwarded request");
	};
	provider
		.setup_request(
			&mut req,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	(llm_request, req)
}

#[tokio::test]
async fn generate_content_keeps_its_route_type_upstream() {
	// The native render's provider format is GenerateContent, so the client-facing
	// route type survives all the way to upstream path building.
	for (provider, host) in [
		(vertex_provider(None, None), "aiplatform.googleapis.com"),
		(gemini_provider(None), gemini::DEFAULT_HOST_STR),
	] {
		let RequestResult::Success {
			llm_request,
			upstream_route_type,
			..
		} = provider
			.process_gemini_request(
				&backend_info(host),
				None,
				generate_content_body("https://example.com/v1beta/models/gemini-2.5-flash:generateContent"),
				false,
				&mut None,
			)
			.await
			.expect("generateContent request should process")
		else {
			panic!("expected a forwarded request");
		};
		assert_eq!(upstream_route_type, RouteType::GenerateContent);
		assert_eq!(llm_request.input_format, InputFormat::Gemini);
		assert!(matches!(
			llm_request.provider_state,
			Some(ProviderState::VertexGemini)
		));
	}
}

#[tokio::test]
async fn completions_inbound_to_the_gemini_provider_renders_native() {
	// The Gemini provider advertises the native chat format unconditionally, so
	// OpenAI-compat clients get the gateway's own Completions -> Gemini conversion
	// instead of Google's compat shim, matching Vertex with a Gemini model.
	let provider = gemini_provider(None);
	let req = gemini_request(
		"https://example.com/v1/chat/completions",
		r#"{"model":"gemini-2.5-flash","messages":[{"role":"user","content":"hello"}]}"#,
	);

	let RequestResult::Success {
		request: mut forwarded,
		llm_request,
		upstream_route_type,
	} = provider
		.process_completions_request(
			&backend_info(gemini::DEFAULT_HOST_STR),
			None,
			req,
			false,
			&mut None,
		)
		.await
		.expect("completions request should process")
	else {
		panic!("expected a forwarded request");
	};

	assert_eq!(upstream_route_type, RouteType::GenerateContent);
	assert!(matches!(
		llm_request.provider_state,
		Some(ProviderState::VertexGemini)
	));

	provider
		.setup_request(
			&mut forwarded,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	assert_eq!(
		forwarded.uri().path(),
		"/v1beta/models/gemini-2.5-flash:generateContent"
	);

	let body = forwarded.into_body().collect().await.unwrap().to_bytes();
	let json: Value = serde_json::from_slice(&body).expect("forwarded body should be JSON");
	assert!(
		json.get("contents").is_some(),
		"native body carries contents: {json}"
	);
	assert!(
		json.get("messages").is_none(),
		"OpenAI messages field must not survive the conversion: {json}"
	);
}

#[test]
fn gemini_input_formats_gate_the_policy_stack() {
	assert!(InputFormat::Gemini.is_chat());
	assert!(InputFormat::Gemini.supports_prompt_guard());
	// countTokens generates no tokens and has no prompt to rewrite, matching the Anthropic
	// token-count route.
	assert!(!InputFormat::GeminiCountTokens.is_chat());
	assert!(!InputFormat::GeminiCountTokens.supports_prompt_guard());
}

#[tokio::test]
async fn backend_model_pinning_rewrites_the_path_model() {
	// The plan's open question: with `ai.provider.vertexai.model` set, the model also rides the
	// client's path, so the override has to reach the upstream path, not just the log.
	let provider = vertex_provider(None, Some("gemini-2.5-pro"));
	let (llm_request, req) = process_and_setup(
		&provider,
		None,
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		"aiplatform.googleapis.com",
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-pro");
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-pro:generateContent"
	);
}

#[tokio::test]
async fn model_alias_rewrites_the_path_model() {
	let provider = vertex_provider(None, None);
	let policy = Policy {
		model_aliases: HashMap::from([(strng::new("fast"), strng::new("gemini-2.5-flash"))]),
		..Default::default()
	};
	let (llm_request, req) = process_and_setup(
		&provider,
		Some(&policy),
		"https://example.com/v1beta/models/fast:streamGenerateContent?alt=sse",
		"aiplatform.googleapis.com",
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert!(llm_request.streaming);
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent"
	);
	assert_eq!(req.uri().query(), Some("alt=sse"));
}

#[tokio::test]
async fn vertex_publisher_shaped_client_path_is_accepted() {
	// LangChain's ChatVertexAI points at the full Vertex resource path rather than the bare
	// Gemini-API one; the model still has to be read out of it.
	let provider = vertex_provider(Some("us-central1"), None);
	let (llm_request, req) = process_and_setup(
		&provider,
		None,
		"https://example.com/v1/projects/other/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent",
		"us-central1-aiplatform.googleapis.com",
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
	);
}

// ── the model segment cannot choose the upstream path ───────────────────────

/// Both spellings of the same attack. `%2F` is the dangerous one: it is not a `/`, so it passes
/// the `[^/]+` route regexes in `binds.rs`, and `Uri::path()` hands it over undecoded — the
/// traversal only exists after `extract_model_from_path` percent-decodes.
const TRAVERSAL_MODELS: &[&str] = &[
	"gemini-2.5-flash/../../../../locations/global/endpoints/openapi/chat/completions",
	"gemini-2.5-flash%2F..%2F..%2F..%2F..%2Flocations%2Fglobal%2Fendpoints%2Fopenapi%2Fchat%2Fcompletions",
];

#[tokio::test]
async fn a_traversal_model_in_the_client_path_is_rejected_before_a_path_is_built() {
	let vertex = vertex_provider(None, None);
	let gemini = gemini_provider(None);
	for model in TRAVERSAL_MODELS {
		let cases = [
			(
				&vertex,
				"aiplatform.googleapis.com",
				format!(
					"https://example.com/v1/projects/other/locations/global/publishers/google/models/{model}:generateContent"
				),
			),
			(
				&gemini,
				gemini::DEFAULT_HOST_STR,
				format!("https://example.com/v1beta/models/{model}:generateContent"),
			),
		];
		for (provider, host, uri) in cases {
			let request = generate_content_body(&uri);
			assert_eq!(
				request.uri().path().contains("%2F"),
				model.contains("%2F"),
				"the client's percent-encoding reaches model extraction undecoded"
			);
			let err = provider
				.process_gemini_request(&backend_info(host), None, request, false, &mut None)
				.await
				.expect_err("a traversal model must not resolve to an upstream request");
			// The model is dropped at extraction and the Gemini body carries none, so the request
			// dies here rather than reaching a path builder.
			assert!(matches!(err, AIError::MissingField(_)), "{uri}: {err:?}");
		}
	}
}

#[tokio::test]
async fn a_configured_traversal_model_is_escaped_rather_than_interpolated() {
	// The model can also come from config (`ai.provider.model`, a model alias), which never went
	// through path extraction, so the path builders have to hold the line themselves.
	let (_, req) = process_and_setup(
		&gemini_provider(Some(TRAVERSAL_MODELS[0])),
		None,
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		gemini::DEFAULT_HOST_STR,
	)
	.await;
	assert_eq!(
		req.uri().path(),
		"/v1beta/models/gemini-2.5-flash%2F..%2F..%2F..%2F..%2Flocations%2Fglobal%2Fendpoints%2Fopenapi%2Fchat%2Fcompletions:generateContent"
	);

	// Vertex has a fixed fallback path to land on, so it uses that rather than escaping.
	let req = setup(
		&vertex_provider(None, None),
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		RouteType::GenerateContent,
		&native_chat_request(TRAVERSAL_MODELS[0], false),
	);
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/global/endpoints/openapi/chat/completions"
	);
}

#[tokio::test]
async fn tuned_model_resource_names_round_trip_to_their_own_collection() {
	// `rewrite_path_model` percent-encodes a virtual-model target's `/` so it stays one segment;
	// decoding it back gives a resource name, and the Gemini API serves those under
	// `/v1beta/tunedModels/{id}`, not under `/v1beta/models/`.
	let provider = gemini_provider(None);
	let uri = "https://example.com/v1beta/models/tunedModels%2Fabc:generateContent";
	let request = generate_content_body(uri);
	assert!(
		request.uri().path().contains("%2F"),
		"the client's percent-encoding must still be intact when the model is extracted"
	);

	let (llm_request, req) = process_and_setup(&provider, None, uri, gemini::DEFAULT_HOST_STR).await;
	assert_eq!(llm_request.request_model, "tunedModels/abc");
	assert_eq!(req.uri().path(), "/v1beta/tunedModels/abc:generateContent");
}

#[tokio::test]
async fn native_request_body_reaches_the_upstream_unchanged() {
	let provider = vertex_provider(None, None);
	let (_, req) = process_and_setup(
		&provider,
		None,
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		"aiplatform.googleapis.com",
	)
	.await;

	let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
		.expect("forwarded body should be JSON");
	assert_eq!(
		body,
		serde_json::json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]})
	);
}

// ── who owns the model ──────────────────────────────────────────────────────

fn model_transformation(expression: &str) -> Policy {
	Policy {
		transformations: Some(HashMap::from([(
			"model".to_string(),
			std::sync::Arc::new(crate::cel::Expression::new_strict(expression).unwrap()),
		)])),
		..Default::default()
	}
}

async fn process_and_setup_count_tokens(
	provider: &AIProvider,
	policy: Option<&Policy>,
	uri: &str,
	body: &str,
) -> (LLMRequest, crate::http::Request) {
	let RequestResult::Success {
		request: mut req,
		llm_request,
		upstream_route_type,
		..
	} = provider
		.process_gemini_count_tokens_request(
			&backend_info("aiplatform.googleapis.com"),
			policy,
			gemini_request(uri, body),
			&mut None,
		)
		.await
		.expect("countTokens request should process")
	else {
		panic!("expected a forwarded request");
	};
	provider
		.setup_request(
			&mut req,
			upstream_route_type,
			Some(&llm_request),
			None,
			None,
			false,
		)
		.expect("setup_request should succeed");
	(llm_request, req)
}

#[tokio::test]
async fn a_model_transformation_rewrites_the_upstream_path_model() {
	// The virtual-model pattern from the local_tests configs: the client asks for `foo/<model>`,
	// `rewrite_path_model` percent-encodes it into the path, and the operator's transformation
	// strips the prefix before it reaches Google. `provider_model_is_set_before_llm_transformations`
	// pins the same precedence on /v1/chat/completions — the gateway injects its model, then the
	// mutation gets the last word — and the Gemini path is rebuilt from the result.
	let policy = model_transformation(r#"llmRequest.model.stripPrefix("foo/")"#);
	let (llm_request, req) = process_and_setup(
		&vertex_provider(None, None),
		Some(&policy),
		"https://example.com/v1beta/models/foo%2Fgemini-2.5-flash:generateContent",
		"aiplatform.googleapis.com",
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/global/publishers/google/models/gemini-2.5-flash:generateContent"
	);
	// The injected model is gateway bookkeeping; the wire body must not grow a `model` field.
	let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
		.expect("forwarded body should be JSON");
	assert_eq!(
		body,
		serde_json::json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]})
	);
}

#[tokio::test]
async fn a_model_override_rewrites_the_upstream_path_model() {
	let policy = Policy {
		overrides: Some(HashMap::from([(
			"model".to_string(),
			serde_json::json!("gemini-2.5-pro"),
		)])),
		..Default::default()
	};
	let (llm_request, req) = process_and_setup(
		&gemini_provider(None),
		Some(&policy),
		"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
		gemini::DEFAULT_HOST_STR,
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-pro");
	assert_eq!(
		req.uri().path(),
		"/v1beta/models/gemini-2.5-pro:generateContent"
	);
}

#[tokio::test]
async fn a_client_body_model_never_outranks_the_path() {
	// Gemini's wire body has no `model`, so one in the body is not a client-facing knob: honouring
	// it would let a request pick a different model than the URI it was routed and authorized on.
	let provider = vertex_provider(None, None);
	let RequestResult::Success {
		request: req,
		llm_request,
		..
	} = provider
		.process_gemini_request(
			&backend_info("aiplatform.googleapis.com"),
			None,
			gemini_request(
				"https://example.com/v1beta/models/gemini-2.5-flash:generateContent",
				r#"{"model":"gemini-2.5-pro","contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
			),
			false,
			&mut None,
		)
		.await
		.expect("generateContent request should process")
	else {
		panic!("expected a forwarded request");
	};

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
		.expect("forwarded body should be JSON");
	assert_eq!(
		body,
		serde_json::json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]})
	);
}

#[tokio::test]
async fn a_model_transformation_rewrites_the_count_tokens_path_model() {
	let policy = model_transformation(r#"llmRequest.model.stripPrefix("foo/")"#);
	let (llm_request, req) = process_and_setup_count_tokens(
		&vertex_provider(Some("us-central1"), None),
		Some(&policy),
		"https://example.com/v1beta/models/foo%2Fgemini-2.5-flash:countTokens",
		r#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	assert_eq!(
		req.uri().path(),
		"/v1/projects/test-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:countTokens"
	);
	let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
		.expect("forwarded body should be JSON");
	assert_eq!(
		body,
		serde_json::json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]})
	);
}

#[tokio::test]
async fn a_count_tokens_body_model_stands_in_when_the_path_has_none() {
	// Unlike generateContent, Vertex countTokens does take a body-level model, and endpoint-shaped
	// paths carry none — so it is the fallback, never an override of the path.
	let (llm_request, req) = process_and_setup_count_tokens(
		&vertex_provider(None, None),
		None,
		"https://example.com/v1/projects/other/locations/global/endpoints/123:countTokens",
		r#"{"model":"gemini-2.5-flash","contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
	)
	.await;

	assert_eq!(llm_request.request_model, "gemini-2.5-flash");
	let body: Value = serde_json::from_slice(&req.into_body().collect().await.unwrap().to_bytes())
		.expect("forwarded body should be JSON");
	assert_eq!(
		body,
		serde_json::json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}]})
	);
}

// ── query handling ──────────────────────────────────────────────────────────

#[test]
fn a_client_alt_query_is_stripped_from_count_tokens() {
	// countTokens is unary, but Google honours `alt=sse` on it and answers with SSE framing that
	// `CountTokensResponse::translate_response` cannot parse — a Google-valid request would become
	// a gateway error.
	for provider in [vertex_provider(None, None), gemini_provider(None)] {
		let req = setup(
			&provider,
			"https://example.com/v1beta/models/gemini-2.5-flash:countTokens?alt=sse&foo=bar",
			RouteType::GeminiCountTokens,
			&count_tokens_request("gemini-2.5-flash"),
		);
		assert_eq!(req.uri().query(), Some("foo=bar"), "{provider:?}");
	}
}

// ── response path selection ─────────────────────────────────────────────────

fn native_translation() -> &'static crate::llm::ChatTranslation {
	vertex_provider(None, None)
		.chat_translation(InputFormat::Gemini, Some("gemini-2.5-flash"))
		.expect("gemini inbound on a gemini upstream")
}

#[test]
fn non_streaming_response_is_forwarded_and_usage_extracted() {
	let body = bytes::Bytes::from_static(
		br#"{"candidates":[{"content":{"role":"model","parts":[{"text":"hi","someNewPartField":1}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":8,"candidatesTokenCount":2,"totalTokenCount":10},"modelVersion":"gemini-2.5-flash-001","someNewTopLevelField":true}"#,
	);
	let resp = native_translation()
		.render_response(
			&body,
			&crate::llm::ChatResponseContext {
				model: "gemini-2.5-flash",
				tool_name_map: None,
			},
		)
		.expect("response should parse");

	let llm = resp.to_llm_response(Default::default());
	assert_eq!(llm.input_tokens, Some(8));
	assert_eq!(llm.total_tokens, Some(10));
	assert_eq!(llm.provider_model.as_deref(), Some("gemini-2.5-flash-001"));

	let out: Value = serde_json::from_slice(&resp.serialize().expect("serialize")).expect("JSON");
	let expected: Value = serde_json::from_slice(&body).expect("JSON");
	assert_eq!(
		out, expected,
		"the response body must reach the client as-is"
	);
}

#[tokio::test]
async fn streaming_response_is_forwarded_byte_for_byte() {
	let sse = concat!(
		"data: {\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"hi\"}]}}]}\n\n",
		"data: {\"candidates\":[{\"finishReason\":\"STOP\"}],",
		"\"usageMetadata\":{\"promptTokenCount\":8,\"candidatesTokenCount\":2,\"totalTokenCount\":10}}\n\n",
	);
	let out = native_translation()
		.stream(
			::http::Response::new(Body::from(sse)),
			crate::llm::ChatStreamContext {
				buffer_limit: 1024 * 1024,
				logger: Default::default(),
				model: "gemini-2.5-flash".to_string(),
				log_content: Default::default(),
				tool_name_map: None,
			},
		)
		.into_body()
		.collect()
		.await
		.expect("collect stream")
		.to_bytes();

	assert_eq!(out.as_ref(), sse.as_bytes());
}

#[tokio::test]
async fn count_tokens_errors_pass_through_unchanged() {
	use crate::llm::BufferedResponse;

	let provider = gemini_provider(None);
	let body = bytes::Bytes::from_static(
		br#"{"error":{"code":404,"message":"model not found","status":"NOT_FOUND"}}"#,
	);
	let mut parts = ::http::Response::new(()).into_parts().0;
	parts.status = ::http::StatusCode::NOT_FOUND;
	let buffered = BufferedResponse {
		parts,
		bytes: body.clone(),
		encoding: None,
	};

	let resp = provider
		.process_gemini_count_tokens_response(
			count_tokens_request("gemini-2.5-flash"),
			buffered,
			None,
			&Default::default(),
		)
		.expect("error response should process");
	assert_eq!(resp.status(), ::http::StatusCode::NOT_FOUND);
	// Only Google upstreams serve this route, so the client already speaks the error shape.
	assert_eq!(resp.into_body().collect().await.unwrap().to_bytes(), body);
}

// ── unsupported upstreams ───────────────────────────────────────────────────

#[tokio::test]
async fn gemini_inbound_requires_a_gemini_upstream() {
	let cases = [
		(
			AIProvider::Anthropic(anthropic::Provider { model: None }),
			"api.anthropic.com",
			"anthropic",
		),
		(
			AIProvider::OpenAI(openai::Provider {
				model: None,
				moderation: None,
			}),
			"api.openai.com",
			"openai",
		),
		(
			AIProvider::Bedrock(crate::llm::BedrockProvider::new(bedrock::Provider {
				model: None,
				region: strng::new("us-east-1"),
				guardrail_identifier: None,
				guardrail_version: None,
			})),
			"bedrock-runtime.us-east-1.amazonaws.com",
			"bedrock",
		),
		// Vertex is Gemini-capable, but only for Gemini models.
		(
			vertex_provider(None, Some("claude-sonnet-4-5")),
			"aiplatform.googleapis.com",
			"vertex",
		),
	];
	for (provider, host, expected) in cases {
		let err = provider
			.process_gemini_request(
				&backend_info(host),
				None,
				generate_content_body("https://example.com/v1beta/models/gemini-2.5-flash:generateContent"),
				false,
				&mut None,
			)
			.await
			.expect_err("Gemini inbound must not reach a non-Gemini upstream");
		assert!(matches!(err, AIError::UnsupportedConversion(_)), "{err}");
		let msg = err.to_string();
		assert!(msg.contains("Gemini") && msg.contains(expected), "{msg}");
	}
}

#[tokio::test]
async fn gemini_count_tokens_requires_a_gemini_upstream() {
	let provider = vertex_provider(None, Some("claude-sonnet-4-5"));
	let req = ::http::Request::builder()
		.uri("https://example.com/v1beta/models/gemini-2.5-flash:countTokens")
		.header(::http::header::CONTENT_TYPE, "application/json")
		.body(Body::from(
			br#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#.to_vec(),
		))
		.unwrap();

	let err = provider
		.process_gemini_count_tokens_request(
			&backend_info("aiplatform.googleapis.com"),
			None,
			req,
			&mut None,
		)
		.await
		.expect_err("countTokens must not reach a non-Gemini upstream");
	assert!(matches!(err, AIError::UnsupportedConversion(_)), "{err}");
	assert!(err.to_string().contains("GeminiCountTokens"), "{err}");
}
