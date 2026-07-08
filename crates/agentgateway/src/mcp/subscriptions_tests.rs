use rmcp::ErrorData;
use rmcp::model::{
	ResourceUpdatedNotification, ResourceUpdatedNotificationParam, SubscriptionsListenResult,
};

use super::*;
use crate::mcp::handler::messages_to_response;

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
	let body = assemble_listen_stream(ack, vec![pipeline], failure_mode);
	read_listen_frames(id, body).await
}

/// Serializes the listen body to its SSE wire form and parses the JSON data frames back out.
async fn read_listen_frames(
	id: RequestId,
	body: futures::stream::BoxStream<'static, Result<ServerJsonRpcMessage, ClientError>>,
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

#[test]
fn validate_listen_filter_normalizes_and_rejects_invalid_shapes() {
	let mut filter = SubscriptionFilter::new()
		.with_resource_subscriptions(vec![])
		.with_tools_list_changed(true);

	validate_listen_filter(&mut filter).unwrap();
	assert_eq!(filter.resource_subscriptions, None);
	assert_eq!(filter.tools_list_changed, Some(true));

	let mut filter = SubscriptionFilter::new().with_resource_subscriptions(vec![]);
	assert_eq!(
		validate_listen_filter(&mut filter).unwrap_err(),
		"subscriptions/listen requires at least one notification filter"
	);
	assert_eq!(filter.resource_subscriptions, None);

	let mut filter = SubscriptionFilter::new()
		.with_resource_subscriptions(vec!["file:///x".to_string()])
		.with_tools_list_changed(true);
	assert_eq!(
		validate_listen_filter(&mut filter).unwrap_err(),
		"subscriptions/listen cannot combine resourceSubscriptions with list-changed flags; open a separate listen stream for each"
	);
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
	let filter =
		SubscriptionFilter::new().with_resource_subscriptions(vec!["http://example.com/a".to_string()]);
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
async fn listen_fail_open_drops_error_and_continues() {
	// The same upstream error frame is dropped under FailOpen; streaming continues.
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
	// ack + both tools frames; the error is dropped and streaming continues.
	assert_eq!(frames.len(), 3);
	assert!(frames.iter().all(|f| f.get("error").is_none()));
	assert_eq!(
		frames
			.iter()
			.filter(|f| f["method"] == "notifications/tools/list_changed")
			.count(),
		2
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
	let body = assemble_listen_stream(
		ack,
		vec![rejecting, Messages::pending()],
		FailureMode::FailClosed,
	);
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

#[test]
fn union_list_changed_ors_categories_across_targets() {
	// The listen ack echoes this union: a category granted on any one target must survive.
	let union = union_list_changed(&[
		("a".to_string(), tools_filter()),
		(
			"b".to_string(),
			SubscriptionFilter::new().with_prompts_list_changed(true),
		),
	]);
	assert_eq!(
		union,
		SubscriptionFilter::new()
			.with_tools_list_changed(true)
			.with_prompts_list_changed(true)
	);
}
