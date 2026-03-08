use rmcp::model::{
	AnnotateAble, Annotated, CallToolRequestParams, CallToolResult, CancelTaskParams,
	CancelTaskResult, ClientResult, CompleteRequestParams, CompleteResult, CompletionInfo,
	CreateElicitationRequest, CreateElicitationRequestParams, CreateTaskResult, ElicitationSchema,
	ErrorCode, ErrorData, GetPromptRequestParams, GetPromptResult, GetTaskInfoParams,
	GetTaskPayloadResult, GetTaskResult, GetTaskResultParams, Implementation, ListPromptsResult,
	ListResourceTemplatesResult, ListResourcesResult, ListTasksResult, ListToolsResult, Meta,
	PaginatedRequestParams, PromptMessage, PromptMessageRole, ProtocolVersion, RawContent,
	RawResource, RawResourceTemplate, ReadResourceRequestParams, ReadResourceResult, Reference,
	ResourceContents, ResourceUpdatedNotification, ResourceUpdatedNotificationParam,
	ServerCapabilities, ServerInfo, ServerNotification, ServerRequest, SubscribeRequestParams,
	TaskStatus, TasksCapability, UnsubscribeRequestParams,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, prompt_router, tool_handler, tool_router};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::oneshot;

use crate::common::task_store::TaskStore;

pub(crate) async fn start_mock_mcp_server(
	label: impl Into<String>,
	stateful: bool,
) -> MockMcpServer {
	start_streamable_mock_server(label, stateful, RobustHandler::new).await
}

pub(crate) async fn start_mock_mcp_meta_server(
	label: impl Into<String>,
	stateful: bool,
) -> MockMcpServer {
	start_streamable_mock_server(label, stateful, MetaOnlyHandler::new).await
}

pub(crate) async fn start_mock_mcp_tools_only_prompt_leak_server(
	label: impl Into<String>,
	stateful: bool,
) -> MockMcpServer {
	start_streamable_mock_server(label, stateful, ToolsOnlyPromptLeakHandler::new).await
}

pub(crate) async fn start_mock_legacy_sse_server() -> MockMcpServer {
	use legacy_rmcp::transport::sse_server::{SseServer, SseServerConfig};
	use tokio_util::sync::CancellationToken;

	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let cancellation = CancellationToken::new();
	let (sse_server, service) = SseServer::new(SseServerConfig {
		bind: addr,
		sse_path: "/sse".to_string(),
		post_path: "/message".to_string(),
		ct: cancellation.child_token(),
		sse_keep_alive: None,
	});

	let (shutdown_tx, shutdown_rx) = oneshot::channel();
	let service_cancellation =
		sse_server.with_service_directly(legacy_sse_mock::LegacyRobustHandler::new);
	let server_task = tokio::spawn(async move {
		let _ = axum::serve(listener, service)
			.with_graceful_shutdown(async move {
				let _ = shutdown_rx.await;
				cancellation.cancel();
				service_cancellation.cancel();
			})
			.await;
	});
	MockMcpServer {
		addr,
		shutdown_tx: Some(shutdown_tx),
		server_task: Some(server_task),
	}
}

pub(crate) struct MockMcpServer {
	pub(crate) addr: std::net::SocketAddr,
	shutdown_tx: Option<oneshot::Sender<()>>,
	server_task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for MockMcpServer {
	fn drop(&mut self) {
		if let Some(tx) = self.shutdown_tx.take() {
			let _ = tx.send(());
		}
		if let Some(task) = self.server_task.take() {
			task.abort();
		}
	}
}

async fn start_streamable_mock_server<H, F>(
	label: impl Into<String>,
	stateful: bool,
	make_handler: F,
) -> MockMcpServer
where
	H: ServerHandler + Clone + Send + Sync + 'static,
	F: Fn(Arc<str>) -> H + Send + Sync + 'static,
{
	use rmcp::transport::StreamableHttpServerConfig;
	use rmcp::transport::streamable_http_server::StreamableHttpService;
	use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;

	let label: Arc<str> = label.into().into();
	let service = StreamableHttpService::new(
		{
			let label = label.clone();
			move || Ok(make_handler(label.clone()))
		},
		LocalSessionManager::default().into(),
		StreamableHttpServerConfig {
			stateful_mode: stateful,
			..Default::default()
		},
	);

	let (shutdown_tx, shutdown_rx) = oneshot::channel();
	let router = axum::Router::new().nest_service("/mcp", service);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let addr = listener.local_addr().unwrap();
	let server_task = tokio::spawn(async move {
		let _ = axum::serve(listener, router)
			.with_graceful_shutdown(async move {
				let _ = shutdown_rx.await;
			})
			.await;
	});
	MockMcpServer {
		addr,
		shutdown_tx: Some(shutdown_tx),
		server_task: Some(server_task),
	}
}

#[derive(Clone)]
struct RobustHandler {
	label: Arc<str>,
	tool_router: rmcp::handler::server::router::tool::ToolRouter<RobustHandler>,
	prompt_router: rmcp::handler::server::router::prompt::PromptRouter<RobustHandler>,
	tasks: Arc<Mutex<TaskStore>>,
}

impl RobustHandler {
	fn new(label: Arc<str>) -> Self {
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
		let req = CreateElicitationRequest::new(params);
		let resp = ctx
			.peer
			.send_request(ServerRequest::CreateElicitationRequest(req))
			.await
			.map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
		if let ClientResult::CreateElicitationResult(res) = resp {
			let mut result =
				CallToolResult::success(vec![Annotated::new(RawContent::text("accepted"), None)]);
			result.structured_content = res.content;
			Ok(result)
		} else {
			Err(ErrorData::internal_error("Unexpected response", None))
		}
	}

	#[rmcp::tool(description = "Trigger Resource Update")]
	async fn trigger_update(
		&self,
		ctx: RequestContext<RoleServer>,
	) -> Result<CallToolResult, ErrorData> {
		let notif =
			ResourceUpdatedNotification::new(ResourceUpdatedNotificationParam::new("memo://data"));
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

	#[rmcp::tool(description = "Return URL elicitation required error for testing")]
	fn test_url_elicitation_required(&self) -> Result<CallToolResult, ErrorData> {
		Err(ErrorData::new(
			ErrorCode::URL_ELICITATION_REQUIRED,
			"This request requires more information.",
			Some(json!({
				"elicitations": [
					{
						"mode": "url",
						"message": "Authenticate to continue",
						"elicitationId": "elicit-1",
						"url": "https://example.com/auth"
					}
				]
			})),
		))
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
		Ok(GetPromptResult::new(vec![PromptMessage::new_text(
			PromptMessageRole::User,
			format!("val: {}", msg),
		)]))
	}
}

#[tool_handler]
#[rmcp::prompt_handler]
impl ServerHandler for RobustHandler {
	fn get_info(&self) -> ServerInfo {
		ServerInfo::new(
			ServerCapabilities::builder()
				.enable_completions()
				.enable_tools()
				.enable_resources()
				.enable_prompts()
				.enable_tasks_with(TasksCapability::server_default())
				.build(),
		)
		.with_protocol_version(ProtocolVersion::V_2025_06_18)
		.with_server_info(Implementation::from_build_env())
	}

	fn list_resources(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ListResourcesResult, ErrorData>> + Send {
		std::future::ready(Ok(ListResourcesResult::with_all_items(vec![
			RawResource::new("memo://data", "data").no_annotation(),
			RawResource::new("memo://{id}", "template").no_annotation(),
		])))
	}

	fn read_resource(
		&self,
		params: ReadResourceRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ReadResourceResult, ErrorData>> + Send {
		std::future::ready(Ok(ReadResourceResult::new(vec![
			ResourceContents::text("server-data", params.uri).with_mime_type("text/plain"),
		])))
	}

	fn list_resource_templates(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + Send {
		std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
			RawResourceTemplate::new("memo://{id}", "template").no_annotation(),
		])))
	}

	fn complete(
		&self,
		request: CompleteRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<CompleteResult, ErrorData>> + Send {
		let kind = match request.r#ref {
			Reference::Prompt(_) => "prompt",
			Reference::Resource(_) => "resource",
		};
		let completion = CompletionInfo::with_all_values(vec![format!(
			"{0}:{kind}:{1}",
			self.label, request.argument.value
		)])
		.map_err(|e| ErrorData::internal_error(e, None));
		std::future::ready(completion.map(CompleteResult::new))
	}

	fn subscribe(
		&self,
		_params: SubscribeRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<(), ErrorData>> + Send {
		std::future::ready(Ok(()))
	}

	fn unsubscribe(
		&self,
		_params: UnsubscribeRequestParams,
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
		Ok(CreateTaskResult::new(task))
	}

	async fn list_tasks(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> Result<ListTasksResult, ErrorData> {
		let tasks = self.tasks.lock().await;
		Ok(ListTasksResult::new(
			tasks.iter_tasks().cloned().collect::<Vec<_>>(),
		))
	}

	async fn get_task_info(
		&self,
		request: GetTaskInfoParams,
		_: RequestContext<RoleServer>,
	) -> Result<GetTaskResult, ErrorData> {
		let mut tasks = self.tasks.lock().await;
		if let Some(entry) = tasks.get_mut(&request.task_id) {
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
		let entry = tasks.get_mut(&request.task_id);
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
			return Ok(GetTaskPayloadResult::new(result));
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
		if let Some(entry) = tasks.get_mut(&request.task_id) {
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

#[derive(Clone)]
struct MetaOnlyHandler {
	label: Arc<str>,
}

impl MetaOnlyHandler {
	fn new(label: Arc<str>) -> Self {
		Self { label }
	}
}

impl ServerHandler for MetaOnlyHandler {
	fn get_info(&self) -> ServerInfo {
		ServerInfo::new(
			ServerCapabilities::builder()
				.enable_completions()
				.enable_tools()
				.build(),
		)
		.with_protocol_version(ProtocolVersion::V_2025_06_18)
		.with_server_info(Implementation::from_build_env())
	}

	fn complete(
		&self,
		request: CompleteRequestParams,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<CompleteResult, ErrorData>> + Send {
		let kind = match request.r#ref {
			Reference::Prompt(_) => "prompt",
			Reference::Resource(_) => "resource",
		};
		let value = format!("meta-{kind}-{}", request.argument.value);
		std::future::ready(
			CompletionInfo::with_all_values(vec![value])
				.map(CompleteResult::new)
				.map_err(|e| ErrorData::internal_error(e, None)),
		)
	}

	fn list_tools(
		&self,
		_: Option<PaginatedRequestParams>,
		_: RequestContext<RoleServer>,
	) -> impl std::future::Future<Output = Result<ListToolsResult, ErrorData>> + Send {
		let mut meta = Meta::new();
		meta.0.insert(
			"label".to_string(),
			serde_json::Value::String(self.label.to_string()),
		);
		let mut result = ListToolsResult::with_all_items(Vec::new());
		result.meta = Some(meta);
		std::future::ready(Ok(result))
	}
}

#[derive(Clone)]
struct ToolsOnlyPromptLeakHandler {
	label: Arc<str>,
	tool_router: rmcp::handler::server::router::tool::ToolRouter<ToolsOnlyPromptLeakHandler>,
	prompt_router: rmcp::handler::server::router::prompt::PromptRouter<ToolsOnlyPromptLeakHandler>,
}

impl ToolsOnlyPromptLeakHandler {
	fn new(label: Arc<str>) -> Self {
		Self {
			label,
			tool_router: Self::tool_router(),
			prompt_router: Self::prompt_router(),
		}
	}
}

#[tool_router]
impl ToolsOnlyPromptLeakHandler {
	#[rmcp::tool(description = "Echo")]
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
}

#[prompt_router]
impl ToolsOnlyPromptLeakHandler {
	#[rmcp::prompt(name = "should_not_leak")]
	fn should_not_leak(
		&self,
		rmcp::handler::server::wrapper::Parameters(val): rmcp::handler::server::wrapper::Parameters<
			serde_json::Value,
		>,
	) -> Result<GetPromptResult, ErrorData> {
		let msg = val.get("val").and_then(|v| v.as_str()).unwrap_or("none");
		Ok(GetPromptResult::new(vec![PromptMessage::new_text(
			PromptMessageRole::User,
			format!("leaked {}: {}", self.label, msg),
		)]))
	}
}

#[tool_handler]
#[rmcp::prompt_handler]
impl ServerHandler for ToolsOnlyPromptLeakHandler {
	fn get_info(&self) -> ServerInfo {
		ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
			.with_protocol_version(ProtocolVersion::V_2025_06_18)
			.with_server_info(Implementation::from_build_env())
	}
}

mod legacy_sse_mock {
	use legacy_rmcp as rmcp;
	use rmcp::handler::server::wrapper::Parameters;
	use rmcp::model::{
		AnnotateAble, CallToolResult, Content, GetPromptRequestParam, GetPromptResult, Implementation,
		ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParam,
		Prompt, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole,
		ProtocolVersion, RawResource, RawResourceTemplate, ReadResourceRequestParam,
		ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo,
	};
	use rmcp::service::RequestContext;
	use rmcp::{ErrorData, RoleServer, ServerHandler, tool_handler, tool_router};
	use serde_json::Value;

	#[derive(Clone)]
	pub struct LegacyRobustHandler {
		tool_router: rmcp::handler::server::router::tool::ToolRouter<LegacyRobustHandler>,
	}

	#[tool_router]
	impl LegacyRobustHandler {
		pub fn new() -> Self {
			Self {
				tool_router: Self::tool_router(),
			}
		}

		#[rmcp::tool(description = "Echo")]
		fn echo(&self, Parameters(val): Parameters<Value>) -> Result<CallToolResult, ErrorData> {
			let text = val.get("val").and_then(|v| v.as_str()).unwrap_or("empty");
			Ok(CallToolResult::success(vec![Content::text(format!(
				"sse: {text}"
			))]))
		}
	}

	#[tool_handler]
	impl ServerHandler for LegacyRobustHandler {
		fn get_info(&self) -> ServerInfo {
			ServerInfo {
				protocol_version: ProtocolVersion::V_2025_06_18,
				capabilities: ServerCapabilities::builder()
					.enable_tools()
					.enable_prompts()
					.enable_resources()
					.build(),
				server_info: Implementation::from_build_env(),
				instructions: None,
			}
		}

		async fn get_prompt(
			&self,
			GetPromptRequestParam { arguments, .. }: GetPromptRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<GetPromptResult, ErrorData> {
			let msg = arguments
				.and_then(|a| a.get("val").cloned())
				.and_then(|v| v.as_str().map(ToString::to_string))
				.unwrap_or_else(|| "none".to_string());
			Ok(GetPromptResult {
				description: None,
				messages: vec![PromptMessage {
					role: PromptMessageRole::User,
					content: PromptMessageContent::text(format!("sse val: {msg}")),
				}],
			})
		}

		async fn list_prompts(
			&self,
			_: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListPromptsResult, ErrorData> {
			Ok(ListPromptsResult {
				prompts: vec![Prompt {
					name: "test_prompt".to_string(),
					title: None,
					description: None,
					icons: None,
					arguments: Some(vec![PromptArgument {
						name: "val".to_string(),
						title: None,
						description: Some("value".to_string()),
						required: Some(false),
					}]),
				}],
				next_cursor: None,
			})
		}

		async fn list_resources(
			&self,
			_: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourcesResult, ErrorData> {
			Ok(ListResourcesResult {
				resources: vec![
					RawResource::new("memo://data", "data").no_annotation(),
					RawResource::new("memo://{id}", "template").no_annotation(),
				],
				next_cursor: None,
			})
		}

		async fn read_resource(
			&self,
			params: ReadResourceRequestParam,
			_: RequestContext<RoleServer>,
		) -> Result<ReadResourceResult, ErrorData> {
			Ok(ReadResourceResult {
				contents: vec![ResourceContents::text("sse-server-data", params.uri)],
			})
		}

		async fn list_resource_templates(
			&self,
			_: Option<PaginatedRequestParam>,
			_: RequestContext<RoleServer>,
		) -> Result<ListResourceTemplatesResult, ErrorData> {
			Ok(ListResourceTemplatesResult {
				next_cursor: None,
				resource_templates: vec![
					RawResourceTemplate {
						uri_template: "memo://{id}".to_string(),
						name: "template".to_string(),
						title: None,
						description: None,
						mime_type: None,
					}
					.no_annotation(),
				],
			})
		}
	}
}
