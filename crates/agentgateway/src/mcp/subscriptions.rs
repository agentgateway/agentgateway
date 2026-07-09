//! `subscriptions/listen` notification filtering.
use rmcp::model::{
	CustomNotification, GetMeta, Meta, RequestId, ServerJsonRpcMessage, ServerNotification,
	SubscriptionFilter, SubscriptionsAcknowledgedNotification,
	SubscriptionsAcknowledgedNotificationParams,
};
use tracing::warn;

use crate::mcp::ClientError;
use crate::mcp::handler::rewrite_resource_update_message;

/// Stamps the subscription id on every forwarded listen notification.
///
/// `ToolListChanged`/`PromptListChanged`/`ResourceListChanged` are rmcp `NotificationNoParam`.
/// Their serializer emits `{"method": ...}` only and drops extension `_meta`.
/// `CustomNotification` serializes extension `Meta` into `params._meta`, which is what
/// `ServerTagsSubscriptionId` checks for on every frame.
/// `ResourceUpdated` round-trips `_meta` through extensions, so it is tagged in place.
///
/// TODO(rmcp fork): `NotificationNoParam::{serialize,deserialize}` (serde_impl.rs) drop
/// `params._meta`. The fork should emit `params: {"_meta": ...}` when extensions carry
/// `Meta` and capture `_meta` into extensions on deserialize, matching `Notification<M,P>`.
/// After the `rmcp` rev is bumped, delete the `CustomNotification` conversion and tag every
/// variant via `GetMeta`.
fn tag_listen_notification(
	message: ServerJsonRpcMessage,
	subscription_id: &RequestId,
) -> Option<ServerJsonRpcMessage> {
	use rmcp::model::{
		ConstString, PromptListChangedNotificationMethod, ResourceListChangedNotificationMethod,
		ToolListChangedNotificationMethod,
	};
	let ServerJsonRpcMessage::Notification(mut jn) = message else {
		return Some(message);
	};
	let replacement = match &mut jn.notification {
		ServerNotification::ResourceUpdatedNotification(_)
		| ServerNotification::CustomNotification(_) => {
			jn.notification
				.get_meta_mut()
				.set_subscription_id(subscription_id.clone());
			None
		},
		ServerNotification::ToolListChangedNotification(_) => Some(custom_tagged_notification(
			ToolListChangedNotificationMethod::VALUE,
			None,
			subscription_id,
		)),
		ServerNotification::PromptListChangedNotification(_) => Some(custom_tagged_notification(
			PromptListChangedNotificationMethod::VALUE,
			None,
			subscription_id,
		)),
		ServerNotification::ResourceListChangedNotification(_) => Some(custom_tagged_notification(
			ResourceListChangedNotificationMethod::VALUE,
			None,
			subscription_id,
		)),
		_ => {
			debug_assert!(
				false,
				"subscriptions/listen forwarded an unhandled notification type"
			);
			warn!("dropping unhandled subscriptions/listen notification");
			return None;
		},
	};
	if let Some(notification) = replacement {
		jn.notification = notification;
	}
	Some(ServerJsonRpcMessage::Notification(jn))
}

fn custom_tagged_notification(
	method: impl Into<String>,
	params: Option<serde_json::Value>,
	subscription_id: &RequestId,
) -> ServerNotification {
	let mut custom = CustomNotification::new(method, params);
	custom
		.get_meta_mut()
		.set_subscription_id(subscription_id.clone());
	ServerNotification::CustomNotification(custom)
}

/// Filters one upstream listen stream and tags the notifications forwarded to the client.
pub(super) fn filter_and_tag_listen_notification(
	message: ServerJsonRpcMessage,
	default_target_name: Option<&String>,
	target: &str,
	filter: &SubscriptionFilter,
	subscription_id: &RequestId,
) -> Option<Result<ServerJsonRpcMessage, ClientError>> {
	let notification = match &message {
		// A listen error means the upstream rejected the subscription. Keep it in the
		// failure-mode path so the client does not get an ack over a silent stream.
		ServerJsonRpcMessage::Error(e) => {
			return Some(Err(ClientError::new(anyhow::anyhow!(
				"upstream '{}' rejected subscriptions/listen: {}",
				target,
				e.error.message
			))));
		},
		// The gateway emits the ack. Drop upstream ack responses and other non-notifications.
		ServerJsonRpcMessage::Notification(n) => n,
		_ => return None,
	};
	let forward = match &notification.notification {
		// Exactly one ack reaches the client: the gateway's synthesized frame 0.
		ServerNotification::SubscriptionsAcknowledgedNotification(_) => false,
		ServerNotification::ToolListChangedNotification(_) => filter.tools_list_changed == Some(true),
		ServerNotification::PromptListChangedNotification(_) => {
			filter.prompts_list_changed == Some(true)
		},
		ServerNotification::ResourceListChangedNotification(_) => {
			filter.resources_list_changed == Some(true)
		},
		ServerNotification::ResourceUpdatedNotification(ru) => filter
			.resource_subscriptions
			.as_ref()
			.is_some_and(|uris| uris.iter().any(|u| u == &ru.params.uri)),
		_ => false,
	};
	if !forward {
		return None;
	}
	let message = rewrite_resource_update_message(default_target_name, target, message);
	tag_listen_notification(message, subscription_id).map(Ok)
}

/// Frame 0 of a listen response: the gateway-synthesized ack. `notifications` echoes the client
/// filter in service+ URI form; `_meta` carries the downstream listen request id.
pub(super) fn synthesize_listen_ack(
	id: RequestId,
	client_filter: SubscriptionFilter,
) -> ServerJsonRpcMessage {
	let mut params = SubscriptionsAcknowledgedNotificationParams::new(client_filter);
	let mut meta = Meta::new();
	meta.set_subscription_id(id);
	params.meta = Some(meta);
	ServerJsonRpcMessage::notification(ServerNotification::SubscriptionsAcknowledgedNotification(
		SubscriptionsAcknowledgedNotification::new(params),
	))
}

#[cfg(test)]
mod tests {
	use futures_util::StreamExt;
	use rmcp::ErrorData;
	use rmcp::model::{
		ResourceUpdatedNotification, ResourceUpdatedNotificationParam, SubscriptionsListenResult,
	};

	use super::*;
	use crate::mcp::FailureMode;
	use crate::mcp::handler::messages_to_response;
	use crate::mcp::mergestream::{MergeStream, Messages};

	fn list_changed(notification: ServerNotification) -> ServerJsonRpcMessage {
		ServerJsonRpcMessage::notification(notification)
	}

	/// An upstream JSON-RPC error frame as it arrives off the wire.
	fn upstream_error(msg: &str) -> ServerJsonRpcMessage {
		ServerJsonRpcMessage::error(ErrorData::internal_error(msg.to_string(), None), None)
	}

	fn tools_list_changed() -> ServerJsonRpcMessage {
		list_changed(ServerNotification::ToolListChangedNotification(
			Default::default(),
		))
	}

	fn resource_updated(uri: &str) -> ServerJsonRpcMessage {
		list_changed(ServerNotification::ResourceUpdatedNotification(
			ResourceUpdatedNotification::new(ResourceUpdatedNotificationParam::new(uri)),
		))
	}

	/// Drives one upstream through the real listen pipeline and extracts the wire JSON frames.
	async fn run_listen(
		id: RequestId,
		client_filter: SubscriptionFilter,
		upstream_filter: SubscriptionFilter,
		upstream_msgs: Vec<Result<ServerJsonRpcMessage, ClientError>>,
		default_target_name: Option<String>,
		target: &str,
		failure_mode: FailureMode,
	) -> Vec<serde_json::Value> {
		let ack = synthesize_listen_ack(id.clone(), client_filter);
		let target_name = agent_core::strng::new(target);
		let target = target.to_string();
		let sub_id = id.clone();
		let pipeline = Messages::from_results(upstream_msgs).filter_map_messages_result(move |msg| {
			filter_and_tag_listen_notification(
				msg,
				default_target_name.as_ref(),
				&target,
				&upstream_filter,
				&sub_id,
			)
		});
		let merged = MergeStream::new_without_merge(vec![(target_name, pipeline)], failure_mode);
		let body = futures::stream::once(async move { Ok(ack) }).chain(merged);
		read_listen_frames(id, body).await
	}

	/// Serializes the listen body to its SSE wire form and parses the JSON data frames back out.
	async fn read_listen_frames(
		id: RequestId,
		body: impl futures_core::Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
	) -> Vec<serde_json::Value> {
		let response = messages_to_response(id, body, None, true).unwrap();
		let bytes = crate::http::read_resp_body(response).await.unwrap();
		let text = std::str::from_utf8(&bytes).unwrap();
		text
			.lines()
			.filter_map(|line| line.strip_prefix("data:"))
			.map(|rest| serde_json::from_str(rest.strip_prefix(' ').unwrap_or(rest)).unwrap())
			.collect()
	}

	fn tools_filter() -> SubscriptionFilter {
		SubscriptionFilter::new().with_tools_list_changed(true)
	}

	#[tokio::test]
	async fn listen_emits_ack_first_then_filtered_notification() {
		let frames = run_listen(
			RequestId::Number(7),
			tools_filter(),
			tools_filter(),
			vec![Ok(tools_list_changed()), Ok(resource_updated("file:///x"))],
			None,
			"svc",
			FailureMode::FailClosed,
		)
		.await;

		// Frame 0 is the synthesized ack carrying the subscription id; the tools notification follows.
		// resource_updated is dropped (no resourceSubscriptions in the filter). Under FailClosed the
		// finite upstream stream ending is anomalous for a long-lived subscription, so the pipeline is
		// torn down with a terminal error frame (frame 2).
		assert_eq!(frames.len(), 3);
		assert_eq!(
			frames[0]["method"], "notifications/subscriptions/acknowledged",
			"frame 0 must be the ack notification"
		);
		assert_eq!(
			frames[0]["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
			7
		);
		assert_eq!(
			frames[0]["params"]["notifications"]["toolsListChanged"], true,
			"the ack must echo the granted filter"
		);
		assert_eq!(frames[1]["method"], "notifications/tools/list_changed");
		assert!(
			frames[2]["error"]["message"]
				.as_str()
				.unwrap()
				.contains("ended"),
			"a premature EOF under FailClosed must send a terminal error"
		);
	}

	#[tokio::test]
	async fn listen_tags_list_changed_frame_on_the_wire() {
		// The tag must survive serialization. NotificationNoParam drops _meta, so the pipeline rebuilds
		// list-changed as CustomNotification. Asserting on the serialized frame is the only way to
		// catch a regression here.
		let frames = run_listen(
			RequestId::String("sub-1".into()),
			tools_filter(),
			tools_filter(),
			vec![Ok(tools_list_changed())],
			None,
			"svc",
			FailureMode::FailOpen,
		)
		.await;

		assert_eq!(frames.len(), 2);
		let tools = &frames[1];
		assert_eq!(tools["method"], "notifications/tools/list_changed");
		assert_eq!(
			tools["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"], "sub-1",
			"list-changed frame must carry the subscription id in params._meta"
		);
	}

	#[tokio::test]
	async fn listen_filter_is_strict_opt_in() {
		// prompts filter + a tools notification => nothing forwarded; output is the ack only.
		let frames = run_listen(
			RequestId::Number(1),
			SubscriptionFilter::new().with_prompts_list_changed(true),
			SubscriptionFilter::new().with_prompts_list_changed(true),
			vec![Ok(tools_list_changed())],
			None,
			"svc",
			FailureMode::FailOpen,
		)
		.await;
		assert_eq!(frames.len(), 1);
		assert_eq!(
			frames[0]["method"],
			"notifications/subscriptions/acknowledged"
		);
	}

	#[tokio::test]
	async fn listen_swallows_upstream_ack_and_response() {
		// Upstream sends its own ack + a SubscriptionsListenResult Response. Exactly one ack (ours)
		// reaches the client, and no method-less Response frame leaks.
		let upstream_ack = synthesize_listen_ack(RequestId::Number(99), tools_filter());
		let upstream_response = ServerJsonRpcMessage::response(
			SubscriptionsListenResult::new(RequestId::Number(99)).into(),
			RequestId::Number(99),
		);
		let frames = run_listen(
			RequestId::Number(1),
			tools_filter(),
			tools_filter(),
			vec![
				Ok(upstream_ack),
				Ok(upstream_response),
				Ok(tools_list_changed()),
			],
			None,
			"svc",
			FailureMode::FailOpen,
		)
		.await;
		let acks = frames
			.iter()
			.filter(|f| f["method"] == "notifications/subscriptions/acknowledged")
			.count();
		assert_eq!(acks, 1, "only the gateway's ack should reach the client");
		assert!(
			frames.iter().all(|f| f["method"].is_string()),
			"no method-less Response frame should leak"
		);
		assert_eq!(
			frames[0]["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
			1
		);
	}

	#[tokio::test]
	async fn listen_multiplex_uri_filter_and_rewrite() {
		// resource_subscriptions holds upstream-form URIs; a matching ResourceUpdated is forwarded,
		// rewritten to service+ form, and tagged. A non-matching URI is dropped.
		let filter = SubscriptionFilter::new()
			.with_resource_subscriptions(vec!["http://example.com/a".to_string()]);
		let frames = run_listen(
			RequestId::Number(5),
			filter.clone(),
			filter,
			vec![
				Ok(resource_updated("http://example.com/a")),
				Ok(resource_updated("http://example.com/b")),
			],
			None,
			"svc",
			FailureMode::FailOpen,
		)
		.await;
		assert_eq!(frames.len(), 2, "ack + the one matching resource update");
		assert_eq!(frames[1]["method"], "notifications/resources/updated");
		assert_eq!(
			frames[1]["params"]["uri"], "svc+http://example.com/a",
			"forwarded URI must be rewritten to service+ multiplex form"
		);
		assert_eq!(
			frames[1]["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
			5
		);
	}

	#[tokio::test]
	async fn listen_fail_closed_surfaces_error_then_ends() {
		// An upstream JSON-RPC error frame must reach the client. Under FailClosed it ends the stream.
		let frames = run_listen(
			RequestId::Number(3),
			tools_filter(),
			tools_filter(),
			vec![
				Ok(tools_list_changed()),
				Ok(upstream_error("boom")),
				Ok(tools_list_changed()),
			],
			None,
			"svc",
			FailureMode::FailClosed,
		)
		.await;
		// ack, one tools frame, then the error frame. The trailing frame is gone.
		assert_eq!(frames.len(), 3);
		assert_eq!(
			frames[0]["method"],
			"notifications/subscriptions/acknowledged"
		);
		assert_eq!(frames[1]["method"], "notifications/tools/list_changed");
		assert!(
			frames[2]["error"]["message"]
				.as_str()
				.unwrap()
				.contains("upstream 'svc' rejected subscriptions/listen: boom")
		);
	}

	#[tokio::test]
	async fn listen_fail_open_drops_error_and_retires_upstream() {
		// The same upstream error frame is dropped under FailOpen; the upstream is retired.
		let frames = run_listen(
			RequestId::Number(3),
			tools_filter(),
			tools_filter(),
			vec![
				Ok(tools_list_changed()),
				Ok(upstream_error("boom")),
				Ok(tools_list_changed()),
			],
			None,
			"svc",
			FailureMode::FailOpen,
		)
		.await;
		// ack + first tools frame; the error is dropped and the trailing frame is gone.
		assert_eq!(frames.len(), 2);
		assert!(frames.iter().all(|f| f.get("error").is_none()));
		assert_eq!(
			frames
				.iter()
				.filter(|f| f["method"] == "notifications/tools/list_changed")
				.count(),
			1
		);
	}

	#[tokio::test]
	async fn listen_fail_closed_first_error_ends_the_merged_stream() {
		// One rejecting pipeline must terminate the whole merged body even while another
		// pipeline is still live; without the cross-pipeline stop this read would hang.
		let id = RequestId::Number(4);
		let ack = synthesize_listen_ack(id.clone(), tools_filter());
		let sub_id = id.clone();
		let rejecting = Messages::from_results(vec![Ok(upstream_error("boom"))])
			.filter_map_messages_result(move |msg| {
				filter_and_tag_listen_notification(msg, None, "svc-a", &tools_filter(), &sub_id)
			});
		let merged = MergeStream::new_without_merge(
			vec![
				("svc-a".into(), rejecting),
				("svc-b".into(), Messages::pending()),
			],
			FailureMode::FailClosed,
		);
		let body = futures::stream::once(async move { Ok(ack) }).chain(merged);
		let frames = tokio::time::timeout(
			std::time::Duration::from_secs(5),
			read_listen_frames(id, body),
		)
		.await
		.expect("the first error must end the stream despite the live pipeline");

		assert_eq!(frames.len(), 2);
		assert_eq!(
			frames[0]["method"],
			"notifications/subscriptions/acknowledged"
		);
		assert!(
			frames[1]["error"]["message"]
				.as_str()
				.unwrap()
				.contains("upstream 'svc-a' rejected subscriptions/listen: boom")
		);
	}
}
