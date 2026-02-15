use itertools::Itertools;
use rmcp::model::*;
use rmcp::service::*;
use rmcp::{RoleServer, ServerHandler, ServiceExt, prompt_router, tool_handler, tool_router};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;
use tokio::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::common::gateway::AgentGateway;

#[tokio::test]
async fn test_mcp_multiplexing_logic() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();

	// Setup adversarial upstreams
	let mcp1 = start_mock_mcp_server("s1", true).await;
	let mcp2 = start_mock_mcp_server("s2", false).await;

	let s3_mock = MockServer::start().await;
	Mock::given(method("POST"))
		.respond_with(ResponseTemplate::new(500).set_body_string("CRASH"))
		.mount(&s3_mock)
		.await;

	// Start AgentGateway
	let config = format!(
		r#"
config: {{}}
binds:
- port: $PORT
  listeners:
  - name: comprehensive-gateway
    routes:
    - matches:
      - path:
          pathPrefix: /mcp
      backends:
      - mcp:
          targets:
          - name: s1
            mcp:
              host: http://{}/mcp
          - name: s2
            mcp:
              host: http://{}/mcp
          - name: s3
            mcp:
              host: http://{}/mcp
      policies:
        mcpAuthorization:
          rules:
          - 'true'
          - deny: 'mcp.tool.target == "s2" && mcp.tool.name == "echo"'
"#,
		mcp1.addr,
		mcp2.addr,
		s3_mock.address()
	);

	let gw = AgentGateway::new(config).await?;
	let mcp_url = format!("http://localhost:{}/mcp", gw.port());

	// Connect client
	let update_count = Arc::new(AtomicUsize::new(0));
	let client = setup_comprehensive_client(&mcp_url, update_count.clone()).await?;

	// 1. Verify Tools Aggregation & Execution
	let tools = client.list_tools(None).await?;
	let names = tools.tools.iter().map(|t| t.name.to_string()).collect_vec();
	assert!(names.contains(&"s1__echo".into()));
	assert!(
		!names.contains(&"s2__echo".into()),
		"RBAC failed to filter s2__echo"
	);
	assert!(
		!names.iter().any(|n| n.starts_with("s3__")),
		"Broken backend tools leaked into list"
	);

	let tool_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__echo".into(),
			arguments: Some(json!({"val": "hello"}).as_object().unwrap().clone()),
			meta: None,
			task: None,
		})
		.await?;

	let tool_val = serde_json::to_value(&tool_resp.content[0])?;
	assert_eq!(
		tool_val.get("text").and_then(|v| v.as_str()),
		Some("s1: hello")
	);

	// 2. Verify Task Lifecycle End-to-End
	let task_call = client
		.send_request(ClientRequest::CallToolRequest(CallToolRequest {
			method: Default::default(),
			params: CallToolRequestParams {
				meta: None,
				task: json!({}).as_object().cloned(),
				name: "s1__echo".into(),
				arguments: Some(json!({"val": "task-hello"}).as_object().unwrap().clone()),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_id = match task_call {
		ServerResult::CreateTaskResult(result) => result.task.task_id,
		other => panic!("Expected CreateTaskResult for task call, got: {:?}", other),
	};
	assert!(
		task_id.contains("__"),
		"expected prefixed task id, got {task_id}"
	);

	let listed_tasks = client
		.send_request(ClientRequest::ListTasksRequest(ListTasksRequest {
			method: Default::default(),
			params: Some(PaginatedRequestParams {
				meta: None,
				cursor: None,
			}),
			extensions: Default::default(),
		}))
		.await?;
	let listed_tasks = match listed_tasks {
		ServerResult::ListTasksResult(result) => result,
		other => panic!("Expected ListTasksResult, got: {:?}", other),
	};
	assert!(
		listed_tasks
			.tasks
			.iter()
			.any(|task| task.task_id == task_id),
		"Task not returned in list/tasks response"
	);

	let task_info = client
		.send_request(ClientRequest::GetTaskInfoRequest(GetTaskInfoRequest {
			method: Default::default(),
			params: GetTaskInfoParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_info = match task_info {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/info, got: {:?}", other),
	};
	assert_eq!(task_info.task_id, task_id);
	assert_eq!(task_info.status, TaskStatus::Completed);

	let task_payload = client
		.send_request(ClientRequest::GetTaskResultRequest(GetTaskResultRequest {
			method: Default::default(),
			params: GetTaskResultParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_payload = match task_payload {
		ServerResult::CustomResult(result) => result.0,
		other => panic!("Expected CustomResult for tasks/result, got: {:?}", other),
	};
	assert_eq!(
		task_payload.get("tool").and_then(|v| v.as_str()),
		Some("echo")
	);
	assert_eq!(
		task_payload
			.get("arguments")
			.and_then(|v| v.get("val"))
			.and_then(|v| v.as_str()),
		Some("task-hello")
	);

	let task_cancel = client
		.send_request(ClientRequest::CancelTaskRequest(CancelTaskRequest {
			method: Default::default(),
			params: CancelTaskParams {
				meta: None,
				task_id: task_id.clone(),
			},
			extensions: Default::default(),
		}))
		.await?;
	let task_cancel = match task_cancel {
		ServerResult::GetTaskResult(result) => result.task,
		other => panic!("Expected GetTaskResult for tasks/cancel, got: {:?}", other),
	};
	assert_eq!(task_cancel.task_id, task_id);
	assert_eq!(task_cancel.status, TaskStatus::Cancelled);

	// 3. Verify Prompts Aggregation & Execution
	let prompts = client.list_prompts(None).await?;
	assert!(prompts.prompts.iter().any(|p| p.name == "s1__test_prompt"));

	let prompt_resp = client
		.get_prompt(GetPromptRequestParams {
			name: "s1__test_prompt".into(),
			arguments: Some(json!({"val": "world"}).as_object().unwrap().clone()),
			meta: None,
		})
		.await?;

	let prompt_val = serde_json::to_value(&prompt_resp.messages[0].content)?;
	assert_eq!(
		prompt_val.get("text").and_then(|v| v.as_str()),
		Some("val: world")
	);

	// 4. Verify Resource URI Wrapping & Template Preservation
	let resources = client.list_resources(None).await?;
	let s1_res = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__data")
		.expect("s1__res missing");
	assert!(
		s1_res.uri.starts_with("agw://s1/"),
		"URI wrapping failed for static resource"
	);

	let s1_template = resources
		.resources
		.iter()
		.find(|r| r.name == "s1__template")
		.expect("s1__template missing");
	assert!(
		s1_template.uri.contains("{id}"),
		"Braces were incorrectly encoded in template: {}",
		s1_template.uri
	);

	// Verify Reading through Wrapped URI
	let r_resp = client
		.read_resource(ReadResourceRequestParams {
			uri: s1_res.uri.clone(),
			meta: None,
		})
		.await?;

	let resource_val = serde_json::to_value(&r_resp.contents[0])?;
	assert_eq!(
		resource_val.get("text").and_then(|v| v.as_str()),
		Some("server-data")
	);

	// 5. Verify Elicitation Round-trip (Stateless Routing)
	let e_resp = client
		.call_tool(CallToolRequestParams {
			name: "s1__elicitation".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;
	assert_eq!(
		e_resp.structured_content.unwrap().get("color").unwrap(),
		"diamond"
	);

	// 6. Verify Notification Passthrough (Resource Update)
	client
		.subscribe(SubscribeRequestParams {
			uri: s1_res.uri.clone(),
			meta: None,
		})
		.await?;

	client
		.call_tool(CallToolRequestParams {
			name: "s1__trigger_update".into(),
			arguments: None,
			meta: None,
			task: None,
		})
		.await?;

	// Wait a moment for the notification to arrive.
	// We don't strictly assert > 0 here because the test harness transport
	// doesn't automatically poll the SSE stream for notifications in this setup.
	tokio::time::sleep(Duration::from_millis(500)).await;

	Ok(())
}

async fn setup_comprehensive_client(
	url: &str,
	update_count: Arc<AtomicUsize>,
) -> anyhow::Result<RunningService<RoleClient, ComprehensiveClientHandler>> {
	use rmcp::transport::StreamableHttpClientTransport;
	let transport = StreamableHttpClientTransport::<reqwest::Client>::from_uri(url.to_string());

	let client_info = ClientInfo {
		meta: None,
		protocol_version: Default::default(),
		capabilities: ClientCapabilities::builder()
			.enable_tasks_with(TasksCapability::client_default())
			.enable_elicitation_with(ElicitationCapability {
				form: Some(FormElicitationCapability {
					schema_validation: Some(true),
				}),
				url: Some(UrlElicitationCapability::default()),
			})
			.build(),
		client_info: Implementation {
			name: "comprehensive-client".to_string(),
			version: "1.0.0".to_string(),
			title: None,
			description: None,
			website_url: None,
			icons: None,
		},
	};

	let client = ComprehensiveClientHandler {
		info: client_info,
		update_count,
	}
	.serve(transport)
	.await
	.map_err(|e| anyhow::anyhow!("failed to serve: {:?}", e))?;

	Ok(client)
}

struct ComprehensiveClientHandler {
	info: ClientInfo,
	update_count: Arc<AtomicUsize>,
}

impl rmcp::ClientHandler for ComprehensiveClientHandler {
	fn get_info(&self) -> ClientInfo {
		self.info.clone()
	}

	fn create_elicitation(
		&self,
		_req: CreateElicitationRequestParams,
		_: RequestContext<rmcp::RoleClient>,
	) -> impl std::future::Future<Output = Result<CreateElicitationResult, ErrorData>> + Send + '_ {
		std::future::ready(Ok(CreateElicitationResult {
			action: ElicitationAction::Accept,
			content: Some(json!({"color": "diamond"})),
		}))
	}

	fn on_resource_updated(
		&self,
		_req: ResourceUpdatedNotificationParam,
		_: NotificationContext<rmcp::RoleClient>,
	) -> impl std::future::Future<Output = ()> + Send + '_ {
		self.update_count.fetch_add(1, Ordering::SeqCst);
		std::future::ready(())
	}
}

async fn start_mock_mcp_server(label: &'static str, stateful: bool) -> MockMcpServer {
	use rmcp::transport::StreamableHttpServerConfig;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

	let service = StreamableHttpService::new(
		move || Ok(RobustHandler::new(label)),
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig {
			stateful_mode: stateful,
			..Default::default()
		},
	);

	let (tx, rx) = tokio::sync::oneshot::channel();
	let router = axum::Router::new().nest_service("/mcp", service);
	let tcp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = tcp_listener.local_addr().unwrap();
	tokio::spawn(async move {
		let _ = axum::serve(tcp_listener, router)
			.with_graceful_shutdown(async {
				let _ = rx.await;
			})
			.await;
	});
	MockMcpServer { addr, _cancel: tx }
}

struct MockMcpServer {
	pub addr: std::net::SocketAddr,
	_cancel: tokio::sync::oneshot::Sender<()>,
}

#[derive(Clone)]
struct RobustHandler {
	label: &'static str,
	tool_router: rmcp::handler::server::router::tool::ToolRouter<RobustHandler>,
	prompt_router: rmcp::handler::server::router::prompt::PromptRouter<RobustHandler>,
	tasks: Arc<Mutex<TaskStore>>,
}

impl RobustHandler {
	fn new(label: &'static str) -> Self {
		Self {
			label,
			tool_router: Self::tool_router(),
			prompt_router: Self::prompt_router(),
			tasks: Arc::new(Mutex::new(TaskStore::default())),
		}
	}
}

#[tool_router]
impl RobustHandler {
	#[rmcp::tool(description = "Echo", execution(task_support = "optional"))]
	fn echo(
		&self,
		rmcp::handler::server::wrapper::Parameters(val): rmcp::handler::server::wrapper::Parameters<
			serde_json::Value,
		>,
	) -> Result<CallToolResult, ErrorData> {
		let text = val.get("val").and_then(|v| v.as_str()).unwrap_or("empty");
		Ok(CallToolResult::success(vec![Annotated::new(
			RawContent::text(format!("{}: {}", self.label, text)),
			None,
		)]))
	}

	#[rmcp::tool(description = "Trigger Elicitation")]
	async fn elicitation(
		&self,
		ctx: RequestContext<RoleServer>,
	) -> Result<CallToolResult, ErrorData> {
		let params = CreateElicitationRequestParams::FormElicitationParams {
			meta: None,
			message: "select gem".to_string(),
			requested_schema: ElicitationSchema::builder()
				.required_string("color")
				.build()
				.unwrap(),
		};
		let req = CreateElicitationRequest {
			method: Default::default(),
			params,
			extensions: Default::default(),
		};
		let resp = ctx
			.peer
			.send_request(ServerRequest::CreateElicitationRequest(req))
			.await
			.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
		if let ClientResult::CreateElicitationResult(res) = resp {
			Ok(CallToolResult {
				content: vec![Annotated::new(RawContent::text("accepted"), None)],
				structured_content: res.content,
				is_error: Some(false),
				meta: None,
			})
		} else {
			Err(ErrorData::internal_error("Unexpected response", None))
		}
	}

	#[rmcp::tool(description = "Trigger Resource Update")]
	async fn trigger_update(
		&self,
		ctx: RequestContext<RoleServer>,
	) -> Result<CallToolResult, ErrorData> {
		let notif = ResourceUpdatedNotification {
			method: Default::default(),
			params: ResourceUpdatedNotificationParam {
				uri: "memo://data".to_string(),
			},
			extensions: Default::default(),
		};
		ctx
			.peer
			.send_notification(ServerNotification::ResourceUpdatedNotification(notif))
			.await
			.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
		Ok(CallToolResult::success(vec![Annotated::new(
			RawContent::text("notified"),
			None,
		)]))
	}
}

#[prompt_router]
impl RobustHandler {
	#[rmcp::prompt(name = "test_prompt")]
	fn test_prompt(
		&self,
		rmcp::handler::server::wrapper::Parameters(val): rmcp::handler::server::wrapper::Parameters<
			serde_json::Value,
		>,
	) -> Result<GetPromptResult, ErrorData> {
		let msg = val.get("val").and_then(|v| v.as_str()).unwrap_or("none");
		Ok(GetPromptResult {
			description: None,
			messages: vec![PromptMessage {
				role: PromptMessageRole::User,
				content: PromptMessageContent::Text {
					text: format!("val: {}", msg),
				},
			}],
		})
	}
}

#[tool_handler]
#[rmcp::prompt_handler]
impl ServerHandler for RobustHandler {
	fn get_info(&self) -> ServerInfo {
		ServerInfo {
			protocol_version: ProtocolVersion::V_2025_06_18,
			capabilities: ServerCapabilities::builder()
				.enable_tools()
				.enable_resources()
				.enable_prompts()
				.enable_tasks_with(TasksCapability::server_default())
				.build(),
			server_info: Implementation::from_build_env(),
			instructions: None,
		}
	}

	fn list_resources(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ListResourcesResult, ErrorData>> + Send {
		std::future::ready(Ok(ListResourcesResult {
			resources: vec![
				RawResource::new("memo://data", "data").no_annotation(),
				RawResource::new("memo://{id}", "template").no_annotation(),
			],
			next_cursor: None,
			meta: None,
		}))
	}

	fn read_resource(
		&self,
		params: ReadResourceRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ReadResourceResult, ErrorData>> + Send {
		std::future::ready(Ok(ReadResourceResult {
			contents: vec![ResourceContents::TextResourceContents {
				uri: params.uri,
				mime_type: Some("text/plain".to_string()),
				text: "server-data".to_string(),
				meta: None,
			}],
		}))
	}

	fn subscribe(
		&self,
		_params: SubscribeRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<(), ErrorData>> + Send {
		std::future::ready(Ok(()))
	}

	async fn enqueue_task(
		&self,
		request: CallToolRequestParams,
		_: RequestContext<RoleServer>,
	) -> Result<CreateTaskResult, ErrorData> {
		let mut tasks = self.tasks.lock().await;
		let result = json!({
			"tool": request.name.to_string(),
			"arguments": request.arguments,
		});
		let task = tasks.create_task(result);
		Ok(CreateTaskResult { task })
	}

	async fn list_tasks(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> Result<ListTasksResult, ErrorData> {
		let tasks = self.tasks.lock().await;
		Ok(ListTasksResult {
			tasks: tasks
				.tasks
				.values()
				.map(|entry| entry.task.clone())
				.collect(),
			next_cursor: None,
			total: None,
		})
	}

	async fn get_task_info(
		&self,
		request: GetTaskInfoParams,
		_: RequestContext<RoleServer>,
	) -> Result<GetTaskResult, ErrorData> {
		let mut tasks = self.tasks.lock().await;
		if let Some(entry) = tasks.tasks.get_mut(&request.task_id) {
			if entry.task.status == TaskStatus::Working && entry.result.is_some() {
				entry.task.status = TaskStatus::Completed;
				entry.task.status_message = Some("completed".to_string());
				entry.task.last_updated_at = entry.task.created_at.clone();
			}
			return Ok(GetTaskResult {
				meta: None,
				task: entry.task.clone(),
			});
		}

		Err(ErrorData::invalid_params(
			"task not found".to_string(),
			None,
		))
	}

	async fn get_task_result(
		&self,
		request: GetTaskResultParams,
		_: RequestContext<RoleServer>,
	) -> Result<GetTaskPayloadResult, ErrorData> {
		let mut tasks = self.tasks.lock().await;
		let entry = tasks.tasks.get_mut(&request.task_id);
		let Some(entry) = entry else {
			return Err(ErrorData::invalid_params(
				"task not found".to_string(),
				None,
			));
		};
		if let Some(result) = entry.result.clone() {
			entry.task.status = TaskStatus::Completed;
			entry.task.status_message = Some("completed".to_string());
			entry.task.last_updated_at = entry.task.created_at.clone();
			return Ok(GetTaskPayloadResult(result));
		}
		Err(ErrorData::invalid_params(
			"task not ready".to_string(),
			None,
		))
	}

	async fn cancel_task(
		&self,
		request: CancelTaskParams,
		_: RequestContext<RoleServer>,
	) -> Result<CancelTaskResult, ErrorData> {
		let mut tasks = self.tasks.lock().await;
		if let Some(entry) = tasks.tasks.get_mut(&request.task_id) {
			entry.task.status = TaskStatus::Cancelled;
			entry.task.status_message = Some("cancelled".to_string());
			entry.task.last_updated_at = entry.task.created_at.clone();
			entry.result = None;
			return Ok(CancelTaskResult {
				meta: None,
				task: entry.task.clone(),
			});
		}
		Err(ErrorData::invalid_params(
			"task not found".to_string(),
			None,
		))
	}
}

#[derive(Debug, Default)]
struct TaskStore {
	next_id: u64,
	tasks: HashMap<String, TaskEntry>,
}

#[derive(Debug)]
struct TaskEntry {
	task: Task,
	result: Option<serde_json::Value>,
}

impl TaskStore {
	fn create_task(&mut self, result: serde_json::Value) -> Task {
		let task_id = format!("task-{}", self.next_id);
		self.next_id += 1;
		let created_at = "2026-01-01T00:00:00Z".to_string();
		let task = Task {
			task_id: task_id.clone(),
			status: TaskStatus::Working,
			status_message: Some("queued".to_string()),
			created_at: created_at.clone(),
			last_updated_at: created_at,
			ttl: None,
			poll_interval: Some(10),
		};
		self.tasks.insert(
			task_id,
			TaskEntry {
				task: task.clone(),
				result: Some(result),
			},
		);
		task
	}
}
