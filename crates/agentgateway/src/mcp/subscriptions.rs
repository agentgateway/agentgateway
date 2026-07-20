//! `subscriptions/listen` notification filtering.
use agent_core::strng::Strng;
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use rmcp::model::{
	CustomNotification, GetMeta, Meta, RequestId, ServerJsonRpcMessage, ServerNotification,
	SubscriptionFilter, SubscriptionsAcknowledgedNotification,
	SubscriptionsAcknowledgedNotificationParams,
};
use tracing::warn;

use crate::mcp::handler::rewrite_resource_messages;
use crate::mcp::mergestream::Messages;
use crate::mcp::{ClientError, FailureMode};

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
		// Preserve upstream JSON-RPC errors so MergeStream can apply the configured failure mode.
		ServerJsonRpcMessage::Error(_) => return Some(Ok(message)),
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
	let message = rewrite_resource_messages(default_target_name, target, message);
	tag_listen_notification(message, subscription_id).map(Ok)
}

pub(super) struct PreparedListenStream {
	pub target: Strng,
	pub stream: Messages,
	pub accepted_filter: SubscriptionFilter,
}

#[derive(Debug)]
pub(super) enum ListenSetupFailure {
	Upstream(Box<ServerJsonRpcMessage>),
	Client(ClientError),
}

impl ListenSetupFailure {
	pub fn into_downstream_message(
		self,
		request_id: RequestId,
	) -> Result<ServerJsonRpcMessage, ClientError> {
		match self {
			Self::Upstream(message) => {
				let ServerJsonRpcMessage::Error(mut error) = *message else {
					unreachable!("only JSON-RPC error frames are retained")
				};
				error.id = Some(request_id);
				Ok(ServerJsonRpcMessage::Error(error))
			},
			Self::Client(error) => Err(error),
		}
	}
}

/// Wait for every selected upstream to acknowledge its listen request and retain its stream.
pub(super) async fn prepare_listen_streams(
	streams: Vec<(Strng, Messages, SubscriptionFilter)>,
	failure_mode: FailureMode,
) -> Result<(Vec<PreparedListenStream>, SubscriptionFilter), ListenSetupFailure> {
	let mut pending = FuturesUnordered::new();
	for (index, (target, mut stream, sent_filter)) in streams.into_iter().enumerate() {
		pending.push(async move {
			let failure_target = target.clone();
			let first = stream.next().await;
			let prepared = match first {
				Some(Ok(ServerJsonRpcMessage::Notification(notification))) => {
					match notification.notification {
						ServerNotification::SubscriptionsAcknowledgedNotification(ack) => {
							Ok(PreparedListenStream {
								target,
								stream,
								accepted_filter: intersect_filters(&sent_filter, &ack.params.notifications),
							})
						},
						_ => Err(ListenSetupFailure::Client(ClientError::new(
							anyhow::anyhow!("upstream {target} sent a non-ack first listen frame"),
						))),
					}
				},
				Some(Ok(ServerJsonRpcMessage::Error(error))) => Err(ListenSetupFailure::Upstream(
					Box::new(ServerJsonRpcMessage::Error(error)),
				)),
				Some(Ok(_)) => Err(ListenSetupFailure::Client(ClientError::new(
					anyhow::anyhow!("upstream {target} sent a non-ack first listen frame"),
				))),
				Some(Err(error)) => Err(ListenSetupFailure::Client(error)),
				None => Err(ListenSetupFailure::Client(ClientError::new(
					anyhow::anyhow!("upstream {target} ended before acknowledging listen"),
				))),
			};
			(index, failure_target, prepared)
		});
	}

	let mut prepared = Vec::new();
	let mut failures = Vec::new();
	while let Some((index, target, result)) = pending.next().await {
		match result {
			Ok(stream) => prepared.push(stream),
			Err(error) if failure_mode == FailureMode::FailClosed => return Err(error),
			Err(error) => {
				warn!(target = %target, ?error, "upstream failed during listen setup, skipping");
				failures.push((index, error));
			},
		}
	}
	if prepared.is_empty() {
		failures.sort_by_key(|(index, _)| *index);
		return Err(
			failures
				.into_iter()
				.next()
				.expect("at least one selected upstream must complete")
				.1,
		);
	}
	let accepted = prepared
		.iter()
		.fold(SubscriptionFilter::new(), |mut union, stream| {
			union_filters(&mut union, &stream.accepted_filter);
			union
		});
	Ok((prepared, accepted))
}

pub(super) struct ResourceSubscription {
	pub owner: String,
	pub client_uri: String,
	pub upstream_uri: String,
}

pub(super) fn client_filter_from_accepted(
	client_filter: &SubscriptionFilter,
	resource_subs: &[ResourceSubscription],
	accepted_filter: &SubscriptionFilter,
	prepared: &[PreparedListenStream],
) -> SubscriptionFilter {
	let mut filter = SubscriptionFilter::new();
	filter.tools_list_changed = (client_filter.tools_list_changed == Some(true)
		&& accepted_filter.tools_list_changed == Some(true))
	.then_some(true);
	filter.prompts_list_changed = (client_filter.prompts_list_changed == Some(true)
		&& accepted_filter.prompts_list_changed == Some(true))
	.then_some(true);
	filter.resources_list_changed = (client_filter.resources_list_changed == Some(true)
		&& accepted_filter.resources_list_changed == Some(true))
	.then_some(true);
	let uris = resource_subs
		.iter()
		.filter(|sub| {
			prepared.iter().any(|stream| {
				stream.target.as_str() == sub.owner
					&& stream
						.accepted_filter
						.resource_subscriptions
						.as_deref()
						.unwrap_or_default()
						.contains(&sub.upstream_uri)
			})
		})
		.map(|sub| sub.client_uri.clone())
		.fold(Vec::new(), |mut uris, uri| {
			if !uris.contains(&uri) {
				uris.push(uri);
			}
			uris
		});
	filter.resource_subscriptions = (!uris.is_empty()).then_some(uris);
	filter
}

fn intersect_filters(
	sent: &SubscriptionFilter,
	acknowledged: &SubscriptionFilter,
) -> SubscriptionFilter {
	let mut filter = SubscriptionFilter::new();
	filter.tools_list_changed = (sent.tools_list_changed == Some(true)
		&& acknowledged.tools_list_changed == Some(true))
	.then_some(true);
	filter.prompts_list_changed = (sent.prompts_list_changed == Some(true)
		&& acknowledged.prompts_list_changed == Some(true))
	.then_some(true);
	filter.resources_list_changed = (sent.resources_list_changed == Some(true)
		&& acknowledged.resources_list_changed == Some(true))
	.then_some(true);
	let acknowledged_uris = acknowledged
		.resource_subscriptions
		.as_deref()
		.unwrap_or_default();
	let uris = sent
		.resource_subscriptions
		.as_deref()
		.unwrap_or_default()
		.iter()
		.filter(|uri| acknowledged_uris.contains(uri))
		.cloned()
		.collect::<Vec<_>>();
	filter.resource_subscriptions = (!uris.is_empty()).then_some(uris);
	filter
}

fn union_filters(union: &mut SubscriptionFilter, filter: &SubscriptionFilter) {
	union.tools_list_changed = (union.tools_list_changed == Some(true)
		|| filter.tools_list_changed == Some(true))
	.then_some(true);
	union.prompts_list_changed = (union.prompts_list_changed == Some(true)
		|| filter.prompts_list_changed == Some(true))
	.then_some(true);
	union.resources_list_changed = (union.resources_list_changed == Some(true)
		|| filter.resources_list_changed == Some(true))
	.then_some(true);
	let resources = union.resource_subscriptions.get_or_insert_default();
	for uri in filter.resource_subscriptions.as_deref().unwrap_or_default() {
		if !resources.contains(uri) {
			resources.push(uri.clone());
		}
	}
	if resources.is_empty() {
		union.resource_subscriptions = None;
	}
}

/// Frame 0 of a listen response: the gateway-synthesized ACK for filters accepted upstream.
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
	fn upstream_error() -> ServerJsonRpcMessage {
		serde_json::from_value(serde_json::json!({
			"jsonrpc": "2.0",
			"id": 7,
			"error": {
				"code": -32042,
				"message": "subscription rejected",
				"data": {"reason": "unavailable"}
			}
		}))
		.unwrap()
	}

	fn tools_list_changed() -> ServerJsonRpcMessage {
		list_changed(ServerNotification::ToolListChangedNotification(
			Default::default(),
		))
	}

	fn prompts_list_changed() -> ServerJsonRpcMessage {
		list_changed(ServerNotification::PromptListChangedNotification(
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
		filters: (SubscriptionFilter, SubscriptionFilter, SubscriptionFilter),
		remaining_msgs: Vec<Result<ServerJsonRpcMessage, ClientError>>,
		default_target_name: Option<String>,
		target: &str,
		failure_mode: FailureMode,
	) -> Vec<serde_json::Value> {
		let (client_filter, upstream_filter, upstream_ack_filter) = filters;
		let target_name = agent_core::strng::new(target);
		let mut upstream_msgs = vec![Ok(synthesize_listen_ack(
			RequestId::Number(99),
			upstream_ack_filter,
		))];
		upstream_msgs.extend(remaining_msgs);
		let (prepared, accepted_filter) = prepare_listen_streams(
			vec![(
				target_name,
				Messages::from_results(upstream_msgs),
				upstream_filter.clone(),
			)],
			failure_mode,
		)
		.await
		.unwrap();
		let resource_subs = client_filter
			.resource_subscriptions
			.as_deref()
			.unwrap_or_default()
			.iter()
			.zip(
				upstream_filter
					.resource_subscriptions
					.as_deref()
					.unwrap_or_default(),
			)
			.map(|(client_uri, upstream_uri)| ResourceSubscription {
				owner: target.to_string(),
				client_uri: client_uri.clone(),
				upstream_uri: upstream_uri.clone(),
			})
			.collect::<Vec<_>>();
		let ack = synthesize_listen_ack(
			id.clone(),
			client_filter_from_accepted(&client_filter, &resource_subs, &accepted_filter, &prepared),
		);
		let PreparedListenStream {
			target: target_name,
			stream,
			accepted_filter,
		} = prepared.into_iter().next().unwrap();
		let target = target.to_string();
		let sub_id = id.clone();
		let pipeline = stream.filter_map_messages_result(move |msg| {
			filter_and_tag_listen_notification(
				msg,
				default_target_name.as_ref(),
				&target,
				&accepted_filter,
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

	fn listen_ack(filter: SubscriptionFilter) -> ServerJsonRpcMessage {
		synthesize_listen_ack(RequestId::Number(99), filter)
	}

	#[tokio::test]
	async fn prepare_listen_streams_unions_accepted_filters() {
		let requested = SubscriptionFilter::new()
			.with_tools_list_changed(true)
			.with_prompts_list_changed(true);
		let (prepared, accepted) = prepare_listen_streams(
			vec![
				(
					"tools".into(),
					Messages::from_results(vec![
						Ok(listen_ack(tools_filter())),
						Ok(tools_list_changed()),
					]),
					requested.clone(),
				),
				(
					"prompts".into(),
					Messages::from_results(vec![
						Ok(listen_ack(
							SubscriptionFilter::new().with_prompts_list_changed(true),
						)),
						Ok(prompts_list_changed()),
					]),
					requested.clone(),
				),
			],
			FailureMode::FailClosed,
		)
		.await
		.unwrap();

		assert_eq!(
			accepted,
			SubscriptionFilter::new()
				.with_tools_list_changed(true)
				.with_prompts_list_changed(true)
		);
		assert_eq!(prepared.len(), 2);
		assert_eq!(
			prepared
				.iter()
				.find(|stream| stream.target == "tools")
				.unwrap()
				.accepted_filter,
			tools_filter()
		);
		assert_eq!(
			prepared
				.iter()
				.find(|stream| stream.target == "prompts")
				.unwrap()
				.accepted_filter,
			SubscriptionFilter::new().with_prompts_list_changed(true)
		);
	}

	fn prepared_with_accepted(
		target: &str,
		accepted_filter: SubscriptionFilter,
	) -> PreparedListenStream {
		PreparedListenStream {
			target: agent_core::strng::new(target),
			stream: Messages::from_results(Vec::new()),
			accepted_filter,
		}
	}

	fn resource_sub(owner: &str, uri: &str) -> ResourceSubscription {
		ResourceSubscription {
			owner: owner.to_string(),
			client_uri: format!("{owner}+{uri}"),
			upstream_uri: uri.to_string(),
		}
	}

	#[test]
	fn client_filter_from_accepted_restores_resource_uri_order() {
		let subs = vec![
			resource_sub("svc", "https://example.com/a"),
			resource_sub("svc", "https://example.com/b"),
			resource_sub("svc", "https://example.com/a"),
		];
		let accepted = SubscriptionFilter::new()
			.with_resource_subscriptions(vec!["https://example.com/a".to_string()]);
		let prepared = [prepared_with_accepted("svc", accepted.clone())];
		assert_eq!(
			client_filter_from_accepted(&SubscriptionFilter::new(), &subs, &accepted, &prepared),
			SubscriptionFilter::new()
				.with_resource_subscriptions(vec!["svc+https://example.com/a".to_string()])
		);
	}

	#[test]
	fn client_filter_from_accepted_distinguishes_same_uri_across_targets() {
		// Both targets were asked for the same upstream-form URI; only `a` accepted it.
		// The ack must keep a's client URI and drop b's.
		let subs = vec![
			resource_sub("a", "https://example.com/r"),
			resource_sub("b", "https://example.com/r"),
		];
		let accepted_a = SubscriptionFilter::new()
			.with_resource_subscriptions(vec!["https://example.com/r".to_string()]);
		let prepared = [
			prepared_with_accepted("a", accepted_a.clone()),
			prepared_with_accepted("b", SubscriptionFilter::new()),
		];
		assert_eq!(
			client_filter_from_accepted(&SubscriptionFilter::new(), &subs, &accepted_a, &prepared),
			SubscriptionFilter::new()
				.with_resource_subscriptions(vec!["a+https://example.com/r".to_string()])
		);
	}

	#[tokio::test]
	async fn prepare_listen_streams_fail_closed_rejects_before_peer_ack() {
		let result = tokio::time::timeout(
			std::time::Duration::from_secs(1),
			prepare_listen_streams(
				vec![
					(
						"rejecting".into(),
						Messages::from_results(vec![Ok(upstream_error())]),
						tools_filter(),
					),
					("pending".into(), Messages::pending(), tools_filter()),
				],
				FailureMode::FailClosed,
			),
		)
		.await
		.expect("first structured rejection must not wait for another upstream");
		assert!(matches!(result, Err(ListenSetupFailure::Upstream(_))));
	}

	#[tokio::test]
	async fn prepare_listen_streams_fail_open_keeps_surviving_ack() {
		let (prepared, accepted) = prepare_listen_streams(
			vec![
				(
					"rejecting".into(),
					Messages::from_results(vec![Ok(upstream_error())]),
					tools_filter(),
				),
				(
					"accepted".into(),
					Messages::from_results(vec![Ok(listen_ack(tools_filter()))]),
					tools_filter(),
				),
			],
			FailureMode::FailOpen,
		)
		.await
		.unwrap();
		assert_eq!(prepared.len(), 1);
		assert_eq!(accepted, tools_filter());
	}

	#[tokio::test]
	async fn listen_emits_ack_first_then_filtered_notification() {
		let frames = run_listen(
			RequestId::Number(7),
			(tools_filter(), tools_filter(), tools_filter()),
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
			(tools_filter(), tools_filter(), tools_filter()),
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
			(
				SubscriptionFilter::new().with_prompts_list_changed(true),
				SubscriptionFilter::new().with_prompts_list_changed(true),
				SubscriptionFilter::new().with_prompts_list_changed(true),
			),
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
		let requested_filter = SubscriptionFilter::new()
			.with_tools_list_changed(true)
			.with_prompts_list_changed(true);
		// The upstream narrows the requested filter. The downstream ACK and notification pipeline
		// must use the accepted subset, not the original request.
		let upstream_response = ServerJsonRpcMessage::response(
			SubscriptionsListenResult::new(RequestId::Number(99)).into(),
			RequestId::Number(99),
		);
		let frames = run_listen(
			RequestId::Number(1),
			(
				requested_filter.clone(),
				requested_filter.clone(),
				tools_filter(),
			),
			vec![Ok(upstream_response), Ok(prompts_list_changed())],
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
		assert_eq!(
			frames[0]["params"]["notifications"],
			serde_json::json!({"toolsListChanged": true})
		);
		assert_eq!(
			frames.len(),
			1,
			"unacknowledged notifications must be dropped"
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
			(filter.clone(), filter.clone(), filter),
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
			(tools_filter(), tools_filter(), tools_filter()),
			vec![
				Ok(tools_list_changed()),
				Ok(upstream_error()),
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
		assert_eq!(frames[2]["error"]["code"], -32042);
		assert_eq!(frames[2]["error"]["message"], "subscription rejected");
		assert_eq!(frames[2]["error"]["data"]["reason"], "unavailable");
	}

	#[tokio::test]
	async fn listen_fail_open_drops_error_and_retires_upstream() {
		// The same upstream error frame is dropped under FailOpen; the upstream is retired.
		let frames = run_listen(
			RequestId::Number(3),
			(tools_filter(), tools_filter(), tools_filter()),
			vec![
				Ok(tools_list_changed()),
				Ok(upstream_error()),
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
		let rejecting =
			Messages::from_results(vec![Ok(upstream_error())]).filter_map_messages_result(move |msg| {
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
		assert!(frames[1].get("error").is_some());
	}
}
