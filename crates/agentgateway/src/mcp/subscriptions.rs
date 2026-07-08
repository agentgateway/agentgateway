//! `subscriptions/listen` notification filtering and stream assembly.

use futures_core::Stream;
use rmcp::model::{
	CustomNotification, GetMeta, Meta, RequestId, ServerJsonRpcMessage, ServerNotification,
	SubscriptionFilter, SubscriptionsAcknowledgedNotification,
	SubscriptionsAcknowledgedNotificationParams,
};
use tracing::warn;

use crate::mcp::handler::rewrite_resource_update_message;
use crate::mcp::mergestream::Messages;
use crate::mcp::{ClientError, FailureMode};

/// Stamps the subscription id on every forwarded listen notification.
///
/// `ToolListChanged`/`PromptListChanged`/`ResourceListChanged` are rmcp `NotificationNoParam`.
/// Their serializer emits `{"method": ...}` only and drops extension `_meta`.
/// `CustomNotification` serializes extension `Meta` into `params._meta`, which is what
/// `ServerTagsSubscriptionId` checks for on every frame.
/// Parameterized notifications like `ResourceUpdated` round-trip `_meta` through extensions, so
/// they are tagged in place.
///
/// TODO(rmcp fork): `NotificationNoParam::{serialize,deserialize}` (serde_impl.rs) drop
/// `params._meta`. The fork should emit `params: {"_meta": ...}` when extensions carry
/// `Meta` and capture `_meta` into extensions on deserialize, matching `Notification<M,P>`.
/// After the `rmcp` rev is bumped, delete the `CustomNotification` conversion and tag every
/// variant via `GetMeta`.
fn tag_listen_notification(
	message: ServerJsonRpcMessage,
	subscription_id: &RequestId,
) -> ServerJsonRpcMessage {
	use rmcp::model::{
		ConstString, PromptListChangedNotificationMethod, ResourceListChangedNotificationMethod,
		ToolListChangedNotificationMethod,
	};
	let ServerJsonRpcMessage::Notification(mut jn) = message else {
		return message;
	};
	let custom_method = match &jn.notification {
		ServerNotification::ToolListChangedNotification(_) => {
			Some(ToolListChangedNotificationMethod::VALUE)
		},
		ServerNotification::PromptListChangedNotification(_) => {
			Some(PromptListChangedNotificationMethod::VALUE)
		},
		ServerNotification::ResourceListChangedNotification(_) => {
			Some(ResourceListChangedNotificationMethod::VALUE)
		},
		_ => None,
	};
	if let Some(method) = custom_method {
		let mut custom = CustomNotification::new(method, None);
		custom
			.get_meta_mut()
			.set_subscription_id(subscription_id.clone());
		jn.notification = ServerNotification::CustomNotification(custom);
	} else {
		jn.notification
			.get_meta_mut()
			.set_subscription_id(subscription_id.clone());
	}
	ServerJsonRpcMessage::Notification(jn)
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
	Some(Ok(tag_listen_notification(message, subscription_id)))
}

pub(super) fn validate_listen_filter(filter: &mut SubscriptionFilter) -> Result<(), &'static str> {
	if filter
		.resource_subscriptions
		.as_ref()
		.is_some_and(Vec::is_empty)
	{
		filter.resource_subscriptions = None;
	}
	// Normalized above: Some(resource_subscriptions) is non-empty.
	let has_uris = filter.resource_subscriptions.is_some();
	if !wants_list_changed(filter) && !has_uris {
		return Err("subscriptions/listen requires at least one notification filter");
	}
	// resourceSubscriptions targets a single upstream, but list-changed flags are gateway-wide.
	// Serving both in one listen would either scope the whole fanout to that one upstream
	// (dropping the others' list-changed) or leak the target's URIs to every upstream.
	if has_uris && wants_list_changed(filter) {
		return Err(
			"subscriptions/listen cannot combine resourceSubscriptions with list-changed flags; open a separate listen stream for each",
		);
	}
	Ok(())
}

pub(super) fn wants_list_changed(filter: &SubscriptionFilter) -> bool {
	filter.tools_list_changed == Some(true)
		|| filter.prompts_list_changed == Some(true)
		|| filter.resources_list_changed == Some(true)
}

/// Union of the granted categories across per-target filters. The ack must not claim a
/// category RBAC stripped from every pipeline, or the client waits on notifications
/// that can never arrive.
pub(super) fn union_list_changed(filters: &[(String, SubscriptionFilter)]) -> SubscriptionFilter {
	let mut union = SubscriptionFilter::new();
	for (_, f) in filters {
		union.tools_list_changed = union.tools_list_changed.or(f.tools_list_changed);
		union.prompts_list_changed = union.prompts_list_changed.or(f.prompts_list_changed);
		union.resources_list_changed = union.resources_list_changed.or(f.resources_list_changed);
	}
	union
}

/// Assembles the listen body with the gateway ack first, then the merged upstream pipelines.
pub(super) fn assemble_listen_stream(
	ack: ServerJsonRpcMessage,
	pipelines: Vec<Messages>,
	failure_mode: FailureMode,
) -> futures::stream::BoxStream<'static, Result<ServerJsonRpcMessage, ClientError>> {
	use futures_util::StreamExt;
	match failure_mode {
		FailureMode::FailOpen => futures::stream::once(async move { Ok(ack) })
			.chain(futures::stream::select_all(pipelines))
			.filter_map(|item| async move {
				match item {
					Ok(m) => Some(Ok(m)),
					Err(e) => {
						warn!(
							"upstream listen stream error, dropping (failure_mode=FailOpen): {}",
							e
						);
						None
					},
				}
			})
			.boxed(),
		FailureMode::FailClosed => {
			// A clean EOF on a long-lived subscription means that upstream stopped sending
			// notifications. FailClosed turns that into one terminal error frame.
			let pipelines = pipelines
				.into_iter()
				.map(|p| {
					p.chain(futures::stream::once(async {
						Err::<ServerJsonRpcMessage, ClientError>(ClientError::new(anyhow::anyhow!(
							"upstream listen stream ended"
						)))
					}))
					.boxed()
				})
				.collect::<Vec<_>>();
			let body = futures::stream::once(async move { Ok(ack) })
				.chain(futures::stream::select_all(pipelines))
				.boxed();
			stop_after_error(body).boxed()
		},
	}
}

/// Ends a listen stream after its first error frame under FailClosed.
fn stop_after_error<S>(stream: S) -> impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>>
where
	S: Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
{
	use futures_util::StreamExt;
	stream.scan(false, |errored, item| {
		if *errored {
			return futures::future::ready(None);
		}
		*errored = item.is_err();
		futures::future::ready(Some(item))
	})
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
#[path = "subscriptions_tests.rs"]
mod subscriptions_tests;
