use anyhow::Context;
use rmcp::ServiceExt;
use rmcp::model::{
	ClientCapabilities, ClientInfo, CreateElicitationRequestParams, CreateElicitationResult,
	ElicitationAction, ElicitationCapability, ErrorData, FormElicitationCapability, Implementation,
	ResourceUpdatedNotificationParam, TasksCapability, UrlElicitationCapability,
};
use rmcp::service::{NotificationContext, RequestContext, RoleClient, RunningService};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) type DefaultClient = RunningService<RoleClient, ()>;
pub(crate) type CapabilityClient = RunningService<RoleClient, CapabilityClientHandler>;

pub(crate) async fn setup_default_client(url: &str) -> anyhow::Result<DefaultClient> {
	use rmcp::transport::StreamableHttpClientTransport;

	let transport = StreamableHttpClientTransport::with_client(
		reqwest::Client::new(),
		rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
			url.to_string(),
		),
	);

	().serve(transport)
		.await
		.context("failed to start default client service")
}

pub(crate) async fn setup_capability_client(
	url: &str,
	update_count: Arc<AtomicUsize>,
) -> anyhow::Result<CapabilityClient> {
	use rmcp::transport::StreamableHttpClientTransport;

	let transport = StreamableHttpClientTransport::with_client(
		reqwest::Client::new(),
		rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig::with_uri(
			url.to_string(),
		),
	);

	let client_info = ClientInfo::new(
		ClientCapabilities::builder()
			.enable_tasks_with(TasksCapability::client_default())
			.enable_elicitation_with(ElicitationCapability {
				form: Some(FormElicitationCapability {
					schema_validation: Some(true),
				}),
				url: Some(UrlElicitationCapability::default()),
			})
			.build(),
		Implementation::new("capability-client", "1.0.0"),
	);

	CapabilityClientHandler {
		info: client_info,
		update_count,
	}
	.serve(transport)
	.await
	.context("failed to start capability client service")
}

pub(crate) struct CapabilityClientHandler {
	info: ClientInfo,
	update_count: Arc<AtomicUsize>,
}

impl rmcp::ClientHandler for CapabilityClientHandler {
	fn get_info(&self) -> ClientInfo {
		self.info.clone()
	}

	fn create_elicitation(
		&self,
		_req: CreateElicitationRequestParams,
		_: RequestContext<RoleClient>,
	) -> impl std::future::Future<Output = Result<CreateElicitationResult, ErrorData>> + Send + '_ {
		std::future::ready(Ok(
			CreateElicitationResult::new(ElicitationAction::Accept)
				.with_content(json!({"color": "diamond"})),
		))
	}

	fn on_resource_updated(
		&self,
		_req: ResourceUpdatedNotificationParam,
		_: NotificationContext<RoleClient>,
	) -> impl std::future::Future<Output = ()> + Send + '_ {
		self.update_count.fetch_add(1, Ordering::SeqCst);
		std::future::ready(())
	}
}
