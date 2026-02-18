use std::net::SocketAddr;

use agent_core::strng;
use itertools::Itertools;
use rmcp::RoleClient;
use rmcp::model::InitializeRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpServerConfig;
use secrecy::SecretString;

use crate::http::auth::BackendAuth;
use crate::http::authorization::{PolicySet, RuleSet};
use crate::mcp::McpAuthorization;
use crate::test_helpers::proxymock::{
	BIND_KEY, TestBind, basic_named_route, basic_route, setup_proxy_test, simple_bind,
};
use crate::types::agent::BackendPolicy;
use crate::*;

#[tokio::test]
async fn stream_to_stream_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn sse_to_stream_single() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, true, false).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn stream_to_sse_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn sse_to_sse_single() {
	let mock = mock_sse_server().await;
	let (_bind, io) = setup_proxy(&mock, true, true).await;
	let client = mcp_sse_client(io).await;
	standard_sse_assertions(client).await;
}

#[tokio::test]
async fn stream_to_multiplex() {
	let mock_stream = mock_streamable_http_server(true).await;
	let mock_sse = mock_sse_server().await;
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_multiplex_mcp_backend(
			"mcp",
			vec![
				("sse", mock_sse.addr, true),
				("mcp", mock_stream.addr, false),
			],
			true,
		)
		.with_bind(simple_bind(basic_named_route(strng::new("/mcp"))));
	let io = t.serve_real_listener(strng::new("bind")).await;
	let client = mcp_streamable_client(io).await;
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.filter(|n| n.contains("decrement") || n.contains("echo"))
		.collect_vec();
	assert_eq!(
		t,
		vec![
			"mcp_decrement".to_string(),
			"mcp_echo".to_string(),
			"mcp_echo_http".to_string(),
			"sse_decrement".to_string(),
			"sse_echo".to_string(),
			"sse_echo_http".to_string()
		]
	);

	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "mcp_echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "sse_echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);

	// No target set...
	assert!(
		client
			.call_tool(rmcp::model::CallToolRequestParams {
				meta: None,
				task: None,
				name: "echo".into(),
				arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
			})
			.await
			.is_err()
	);
}

#[tokio::test]
async fn stateless_to_stateful() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn stateless_to_stateless() {
	let mock = mock_streamable_http_server(false).await;
	let (_bind, io) = setup_proxy(&mock, false, false).await;
	let client = mcp_streamable_client(io).await;
	standard_assertions(client).await;
}

#[tokio::test]
async fn stream_to_stream_single_tls() {
	let mock = mock_streamable_http_server(true).await;
	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::BackendAuth(BackendAuth::Key(
			SecretString::new("my-key".into()),
		))],
	)
	.await;
	let client = mcp_streamable_client(io).await;
	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "echo_http".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"Bearer my-key"#
	);
}

/// Test that calling a tool denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown tool: {tool_name}"
#[tokio::test]
async fn authorization_denied_returns_unknown_tool_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all tools
	// The deny rule matches all tools; no allow rules means everything is denied
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to call a tool - should fail with "Unknown tool" error
	let result = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected tool call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				"Unknown tool: echo",
				"Expected error message 'Unknown tool: echo', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

/// Test that getting a prompt denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown prompt: {prompt_name}"
#[tokio::test]
async fn authorization_denied_returns_unknown_prompt_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all prompts
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to get a prompt - should fail with "Unknown prompt" error
	let result = client
		.get_prompt(rmcp::model::GetPromptRequestParams {
			meta: None,
			name: "example_prompt".into(),
			arguments: None,
		})
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected get_prompt call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				"Unknown prompt: example_prompt",
				"Expected error message 'Unknown prompt: example_prompt', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

/// Test that reading a resource denied by MCP authorization policy returns proper JSON-RPC error
/// with INVALID_PARAMS error code (-32602) and message "Unknown resource: {resource_uri}"
#[tokio::test]
async fn authorization_denied_returns_unknown_resource_error() {
	let mock = mock_streamable_http_server(true).await;

	// Create an MCP authorization policy that denies all resources
	let deny_all_policy = McpAuthorization::new(RuleSet::new(PolicySet::new(
		vec![],                                                       // no allow rules
		vec![Arc::new(cel::Expression::new_strict("true").unwrap())], // deny all
	)));

	let (_bind, io) = setup_proxy_policies(
		&mock,
		true,
		false,
		vec![BackendPolicy::McpAuthorization(deny_all_policy)],
	)
	.await;

	let client = mcp_streamable_client(io).await;

	// Attempt to read a resource - should fail with "Unknown resource" error
	let result = client
		.read_resource(rmcp::model::ReadResourceRequestParams {
			meta: None,
			uri: "memo://insights".into(),
		})
		.await;

	// The call should fail
	assert!(
		result.is_err(),
		"Expected read_resource call to fail due to authorization denial"
	);

	let err = result.unwrap_err();

	// Verify error code is INVALID_PARAMS (-32602) and message format
	match &err {
		rmcp::ServiceError::McpError(mcp_error) => {
			assert_eq!(
				mcp_error.code.0, -32602,
				"Expected INVALID_PARAMS error code (-32602), got: {}",
				mcp_error.code.0
			);
			assert_eq!(
				mcp_error.message.as_ref(),
				"Unknown resource: memo://insights",
				"Expected error message 'Unknown resource: memo://insights', got: {}",
				mcp_error.message
			);
		},
		other => panic!("Expected ServiceError::McpError, got: {:?}", other),
	}
}

async fn standard_assertions(client: RunningService<RoleClient, InitializeRequestParams>) {
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.take(2)
		.collect_vec();
	assert_eq!(t, vec!["decrement".to_string(), "echo".to_string()]);
	let ctr = client
		.call_tool(rmcp::model::CallToolRequestParams {
			meta: None,
			task: None,
			name: "echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);
}

async fn standard_sse_assertions(client: LegacyService) {
	let tools = client.list_tools(None).await.unwrap();
	let t = tools
		.tools
		.into_iter()
		.map(|t| t.name.to_string())
		.sorted()
		.take(2)
		.collect_vec();
	assert_eq!(t, vec!["decrement".to_string(), "echo".to_string()]);
	let ctr = client
		.call_tool(legacy_rmcp::model::CallToolRequestParam {
			name: "echo".into(),
			arguments: serde_json::json!({"hi": "world"}).as_object().cloned(),
		})
		.await
		.unwrap();
	assert_eq!(
		&ctr.content[0].raw.as_text().unwrap().text,
		r#"{"hi":"world"}"#
	);
}

async fn setup_proxy(
	mock: &MockServer,
	stateful: bool,
	legacy_sse: bool,
) -> (TestBind, SocketAddr) {
	setup_proxy_policies(mock, stateful, legacy_sse, vec![]).await
}

async fn setup_proxy_policies(
	mock: &MockServer,
	stateful: bool,
	legacy_sse: bool,
	policies: Vec<BackendPolicy>,
) -> (TestBind, SocketAddr) {
	let t = setup_proxy_test("{}")
		.unwrap()
		.with_mcp_backend_policies(mock.addr, stateful, legacy_sse, policies)
		.with_bind(simple_bind(basic_route(mock.addr)));
	let io = t.serve_real_listener(BIND_KEY).await;
	(t, io)
}

pub async fn mcp_streamable_client(
	s: SocketAddr,
) -> RunningService<RoleClient, InitializeRequestParams> {
	use rmcp::ServiceExt;
	use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use rmcp::transport::StreamableHttpClientTransport;
	let transport =
		StreamableHttpClientTransport::<reqwest::Client>::from_uri(format!("http://{s}/mcp"));
	let client_info = ClientInfo {
		meta: None,
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::default(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			website_url: None,
			icons: None,
			description: None,
		},
	};

	client_info
		.serve(transport)
		.await
		.inspect_err(|e| {
			tracing::error!("client error: {:?}", e);
		})
		.unwrap()
}

type LegacyService = legacy_rmcp::service::RunningService<
	legacy_rmcp::RoleClient,
	legacy_rmcp::model::InitializeRequestParam,
>;

pub async fn mcp_sse_client(s: SocketAddr) -> LegacyService {
	use legacy_rmcp::ServiceExt;
	use legacy_rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
	use legacy_rmcp::transport::SseClientTransport;
	let transport = SseClientTransport::<legacyreqwest::Client>::start(format!("http://{s}/sse"))
		.await
		.unwrap();
	let client_info = ClientInfo {
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::default(),
		client_info: Implementation {
			name: "test client".to_string(),
			version: "0.0.1".to_string(),
			title: None,
			website_url: None,
			icons: None,
		},
	};

	client_info.serve(transport).await.unwrap()
}

struct MockServer {
	addr: SocketAddr,
	_cancel: tokio::sync::oneshot::Sender<()>,
}

async fn mock_streamable_http_server(stateful: bool) -> MockServer {
	use mockserver::Counter;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
	agent_core::telemetry::testing::setup_test_logging();

	let service = StreamableHttpService::new(
		|| Ok(Counter::new()),
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig {
			sse_retry: None,
			sse_keep_alive: None,
			stateful_mode: stateful,
			cancellation_token: Default::default(),
		},
	);

	let (tx, rx) = tokio::sync::oneshot::channel();
	let router = axum::Router::new().nest_service("/mcp", service);
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, router)
			.with_graceful_shutdown(async { rx.await.unwrap() })
			.await;
		info!("server stopped");
	});
	MockServer { addr, _cancel: tx }
}

async fn mock_sse_server() -> MockServer {
	use legacy_rmcp::transport::sse_server::{SseServer, SseServerConfig};
	use tokio_util::sync::CancellationToken;

	agent_core::telemetry::testing::setup_test_logging();
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	let ct = CancellationToken::new();
	let (sse_server, service) = SseServer::new(SseServerConfig {
		bind: addr,
		sse_path: "/sse".to_string(),
		post_path: "/message".to_string(),
		ct: ct.child_token(),
		sse_keep_alive: None,
	});

	let (tx, rx) = tokio::sync::oneshot::channel();
	let ct2 = sse_server.with_service_directly(legacymockserver::Counter::new);
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, service)
			.with_graceful_shutdown(async move {
				rx.await.unwrap();
				ct.cancel();
				ct2.cancel();
				tracing::info!("sse server cancelled");
			})
			.await;
	});
	MockServer { addr, _cancel: tx }
}
mod mockserver {
	use std::sync::Arc;

	use http::request::Parts;
	use rmcp::handler::server::router::prompt::PromptRouter;
	use rmcp::handler::server::router::tool::ToolRouter;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::*;
	use rmcp::service::RequestContext;
	use rmcp::{
		ErrorData as McpError, RoleServer, ServerHandler, prompt, prompt_handler, prompt_router,
		schemars, tool, tool_handler, tool_router,
	};
	use serde_json::json;
	use tokio::sync::Mutex;

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct ExamplePromptArgs {
		/// A message to put in the prompt
		pub message: String,
	}

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct CounterAnalysisArgs {
		/// The target value you're trying to reach
		pub goal: i32,
		/// Preferred strategy: 'fast' or 'careful'
		#[serde(skip_serializing_if = "Option::is_none")]
		pub strategy: Option<String>,
	}

	#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
	pub struct StructRequest {
		pub a: i32,
		pub b: i32,
	}

	#[derive(Clone)]
	pub struct Counter {
		counter: Arc<Mutex<i32>>,
		tool_router: ToolRouter<Counter>,
		prompt_router: PromptRouter<Counter>,
	}

	#[tool_router]
	impl Counter {
		#[allow(dead_code)]
		pub fn new() -> Self {
			Self {
				counter: Arc::new(Mutex::new(0)),
				tool_router: Self::tool_router(),
				prompt_router: Self::prompt_router(),
			}
		}

		fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
			RawResource::new(uri, name.to_string()).no_annotation()
		}

		#[tool(description = "Increment the counter by 1")]
		async fn increment(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter += 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Decrement the counter by 1")]
		async fn decrement(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter -= 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Get the current counter value")]
		async fn get_value(&self) -> Result<CallToolResult, McpError> {
			let counter = self.counter.lock().await;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Say hello to the client")]
		fn say_hello(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("hello")]))
		}

		#[tool(description = "Repeat what you say")]
		fn echo(&self, Parameters(object): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				serde_json::Value::Object(object).to_string(),
			)]))
		}

		#[tool(description = "Calculate the sum of two numbers")]
		fn sum(
			&self,
			Parameters(StructRequest { a, b }): Parameters<StructRequest>,
		) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				(a + b).to_string(),
			)]))
		}

		#[tool(description = "Echo HTTP attributes")]
		fn echo_http(&self, rq: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
			let ext = rq.extensions.get::<Parts>();
			Ok(CallToolResult::success(vec![Content::text(
				ext
					.unwrap()
					.headers
					.get("authorization")
					.map(|s| String::from_utf8_lossy(s.as_bytes()))
					.unwrap_or_default(),
			)]))
		}
	}

	#[prompt_router]
	impl Counter {
		/// This is an example prompt that takes one required argument, message
		#[prompt(name = "example_prompt")]
		async fn example_prompt(
			&self,
			Parameters(args): Parameters<ExamplePromptArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<Vec<PromptMessage>, McpError> {
			let prompt = format!(
				"This is an example prompt with your message here: '{}'",
				args.message
			);
			Ok(vec![PromptMessage {
				role: PromptMessageRole::User,
				content: PromptMessageContent::text(prompt),
			}])
		}

		/// Analyze the current counter value and suggest next steps
		#[prompt(name = "counter_analysis")]
		async fn counter_analysis(
			&self,
			Parameters(args): Parameters<CounterAnalysisArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<GetPromptResult, McpError> {
			let strategy = args.strategy.unwrap_or_else(|| "careful".to_string());
			let current_value = *self.counter.lock().await;
			let difference = args.goal - current_value;

			let messages = vec![
				PromptMessage::new_text(
					PromptMessageRole::Assistant,
					"I'll analyze the counter situation and suggest the best approach.",
				),
				PromptMessage::new_text(
					PromptMessageRole::User,
					format!(
						"Current counter value: {}\nGoal value: {}\nDifference: {}\nStrategy preference: {}\n\nPlease analyze the situation and suggest the best approach to reach the goal.",
						current_value, args.goal, difference, strategy
					),
				),
			];

			Ok(GetPromptResult {
				description: Some(format!(
					"Counter analysis for reaching {} from {}",
					args.goal, current_value
				)),
				messages,
			})
		}
	}

	#[tool_handler]
	#[prompt_handler]
	impl ServerHandler for Counter {
		fn get_info(&self) -> ServerInfo {
			ServerInfo {
				protocol_version: ProtocolVersion::V_2025_06_18,
				capabilities: ServerCapabilities::builder()
					.enable_prompts()
					.enable_resources()
					.enable_tools()
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: Some("This server provides counter tools and prompts.".to_string()),
			}
		}

		async fn list_resources(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourcesResult, McpError> {
			Ok(ListResourcesResult {
				resources: vec![
					self._create_resource_text("str:////Users/to/some/path/", "cwd"),
					self._create_resource_text("memo://insights", "memo-name"),
				],
				next_cursor: None,
				meta: None,
			})
		}

		async fn read_resource(
			&self,
			ReadResourceRequestParams { uri, .. }: ReadResourceRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<ReadResourceResult, McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" => {
					let cwd = "/Users/to/some/path/";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(cwd, uri)],
					})
				},
				"memo://insights" => {
					let memo = "Business Intelligence Memo\n\nAnalysis has revealed 5 key insights ...";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(memo, uri)],
					})
				},
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn list_resource_templates(
			&self,
			_request: Option<PaginatedRequestParams>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, McpError> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: Vec::new(),
				meta: None,
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParams,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			Ok(self.get_info())
		}
	}
}

mod legacymockserver {
	use std::sync::Arc;

	use http::request::Parts;
	use legacy_rmcp as rmcp;
	use rmcp::handler::server::router::prompt::PromptRouter;
	use rmcp::handler::server::router::tool::ToolRouter;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::*;
	use rmcp::service::RequestContext;
	use rmcp::{
		ErrorData as McpError, RoleServer, ServerHandler, prompt, prompt_handler, prompt_router,
		schemars, tool, tool_handler, tool_router,
	};
	use serde_json::json;
	use tokio::sync::Mutex;

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct ExamplePromptArgs {
		/// A message to put in the prompt
		pub message: String,
	}

	#[derive(Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
	pub struct CounterAnalysisArgs {
		/// The target value you're trying to reach
		pub goal: i32,
		/// Preferred strategy: 'fast' or 'careful'
		#[serde(skip_serializing_if = "Option::is_none")]
		pub strategy: Option<String>,
	}

	#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
	pub struct StructRequest {
		pub a: i32,
		pub b: i32,
	}

	#[derive(Clone)]
	pub struct Counter {
		counter: Arc<Mutex<i32>>,
		tool_router: ToolRouter<Counter>,
		prompt_router: PromptRouter<Counter>,
	}

	#[tool_router]
	impl Counter {
		#[allow(dead_code)]
		pub fn new() -> Self {
			Self {
				counter: Arc::new(Mutex::new(0)),
				tool_router: Self::tool_router(),
				prompt_router: Self::prompt_router(),
			}
		}

		fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
			RawResource::new(uri, name.to_string()).no_annotation()
		}

		#[tool(description = "Increment the counter by 1")]
		async fn increment(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter += 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Decrement the counter by 1")]
		async fn decrement(&self) -> Result<CallToolResult, McpError> {
			let mut counter = self.counter.lock().await;
			*counter -= 1;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Get the current counter value")]
		async fn get_value(&self) -> Result<CallToolResult, McpError> {
			let counter = self.counter.lock().await;
			Ok(CallToolResult::success(vec![Content::text(
				counter.to_string(),
			)]))
		}

		#[tool(description = "Say hello to the client")]
		fn say_hello(&self) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text("hello")]))
		}

		#[tool(description = "Repeat what you say")]
		fn echo(&self, Parameters(object): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				serde_json::Value::Object(object).to_string(),
			)]))
		}

		#[tool(description = "Calculate the sum of two numbers")]
		fn sum(
			&self,
			Parameters(StructRequest { a, b }): Parameters<StructRequest>,
		) -> Result<CallToolResult, McpError> {
			Ok(CallToolResult::success(vec![Content::text(
				(a + b).to_string(),
			)]))
		}

		#[tool(description = "Echo HTTP attributes")]
		fn echo_http(&self, rq: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
			let ext = rq.extensions.get::<Parts>();
			Ok(CallToolResult::success(vec![Content::text(
				ext
					.unwrap()
					.headers
					.get("authorization")
					.map(|s| String::from_utf8_lossy(s.as_bytes()))
					.unwrap_or_default(),
			)]))
		}
	}

	#[prompt_router]
	impl Counter {
		/// This is an example prompt that takes one required argument, message
		#[prompt(name = "example_prompt")]
		async fn example_prompt(
			&self,
			Parameters(args): Parameters<ExamplePromptArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<Vec<PromptMessage>, McpError> {
			let prompt = format!(
				"This is an example prompt with your message here: '{}'",
				args.message
			);
			Ok(vec![PromptMessage {
				role: PromptMessageRole::User,
				content: PromptMessageContent::text(prompt),
			}])
		}

		/// Analyze the current counter value and suggest next steps
		#[prompt(name = "counter_analysis")]
		async fn counter_analysis(
			&self,
			Parameters(args): Parameters<CounterAnalysisArgs>,
			_ctx: RequestContext<RoleServer>,
		) -> Result<GetPromptResult, McpError> {
			let strategy = args.strategy.unwrap_or_else(|| "careful".to_string());
			let current_value = *self.counter.lock().await;
			let difference = args.goal - current_value;

			let messages = vec![
				PromptMessage::new_text(
					PromptMessageRole::Assistant,
					"I'll analyze the counter situation and suggest the best approach.",
				),
				PromptMessage::new_text(
					PromptMessageRole::User,
					format!(
						"Current counter value: {}\nGoal value: {}\nDifference: {}\nStrategy preference: {}\n\nPlease analyze the situation and suggest the best approach to reach the goal.",
						current_value, args.goal, difference, strategy
					),
				),
			];

			Ok(GetPromptResult {
				description: Some(format!(
					"Counter analysis for reaching {} from {}",
					args.goal, current_value
				)),
				messages,
			})
		}
	}

	#[tool_handler]
	#[prompt_handler]
	impl ServerHandler for Counter {
		fn get_info(&self) -> ServerInfo {
			ServerInfo {
				protocol_version: ProtocolVersion::V_2025_06_18,
				capabilities: ServerCapabilities::builder()
					.enable_prompts()
					.enable_resources()
					.enable_tools()
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: Some("This server provides counter tools and prompts.".to_string()),
			}
		}

		async fn list_resources(
			&self,
			_request: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourcesResult, McpError> {
			Ok(ListResourcesResult {
				resources: vec![
					self._create_resource_text("str:////Users/to/some/path/", "cwd"),
					self._create_resource_text("memo://insights", "memo-name"),
				],
				next_cursor: None,
			})
		}

		async fn read_resource(
			&self,
			ReadResourceRequestParam { uri }: ReadResourceRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<ReadResourceResult, McpError> {
			match uri.as_str() {
				"str:////Users/to/some/path/" => {
					let cwd = "/Users/to/some/path/";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(cwd, uri)],
					})
				},
				"memo://insights" => {
					let memo = "Business Intelligence Memo\n\nAnalysis has revealed 5 key insights ...";
					Ok(ReadResourceResult {
						contents: vec![ResourceContents::text(memo, uri)],
					})
				},
				_ => Err(McpError::resource_not_found(
					"resource_not_found",
					Some(json!({
							"uri": uri
					})),
				)),
			}
		}

		async fn list_resource_templates(
			&self,
			_request: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, McpError> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: Vec::new(),
			})
		}

		async fn initialize(
			&self,
			_request: InitializeRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<InitializeResult, McpError> {
			Ok(self.get_info())
		}
	}
}
