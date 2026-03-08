use super::*;
use agent_core::strng;
use rstest::rstest;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tempfile::NamedTempFile;

fn capture_target(name: &str, capture_file: &Path) -> Arc<crate::mcp::router::McpTarget> {
	Arc::new(crate::mcp::router::McpTarget {
		name: name.into(),
		spec: crate::types::agent::McpTargetSpec::Stdio {
			cmd: "sh".into(),
			args: vec![
				"-c".into(),
				"while IFS= read -r line; do printf '%s\\n' \"$line\" >> \"$CAPTURE_FILE\"; done".into(),
			],
			env: HashMap::from([(
				"CAPTURE_FILE".to_string(),
				capture_file.display().to_string(),
			)]),
		},
		backend: None,
		always_use_prefix: false,
		backend_policies: Default::default(),
	})
}

async fn wait_until_contains(path: &Path, needle: &str) {
	let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
	loop {
		let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
		if content.contains(needle) {
			return;
		}
		if tokio::time::Instant::now() >= deadline {
			panic!(
				"capture {:?} did not contain {:?}; last content: {:?}",
				path, needle, content
			);
		}
		tokio::time::sleep(Duration::from_millis(20)).await;
	}
}

async fn assert_not_contains_for(path: &Path, needle: &str, duration: Duration) {
	let deadline = tokio::time::Instant::now() + duration;
	loop {
		let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
		if content.contains(needle) {
			panic!(
				"capture {:?} unexpectedly contained {:?}; content: {:?}",
				path, needle, content
			);
		}
		if tokio::time::Instant::now() >= deadline {
			return;
		}
		tokio::time::sleep(Duration::from_millis(20)).await;
	}
}

fn single_capture_relay(allow_degraded: bool, capture_file: &Path) -> Result<Relay, mcp::Error> {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	Relay::new(
		McpBackendGroup {
			targets: vec![capture_target("serverA", capture_file)],
			stateful: false,
			allow_degraded,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
}

#[test]
fn merge_meta_includes_upstreams() {
	let mut meta_a = Meta::new();
	meta_a.0.insert("a".to_string(), json!(1));
	let mut meta_b = Meta::new();
	meta_b.0.insert("b".to_string(), json!(2));
	let merged = merge_meta(vec![
		(strng::new("a"), Some(meta_a)),
		(strng::new("b"), Some(meta_b)),
	])
	.expect("merged meta");
	let upstreams = merged
		.0
		.get("upstreams")
		.and_then(|v| v.as_object())
		.expect("meta.upstreams");
	assert!(upstreams.contains_key("a"));
	assert!(upstreams.contains_key("b"));
}

#[tokio::test]
async fn merge_tasks_drops_pagination_when_multiple_upstreams_participate() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let mut a = ListTasksResult::new(vec![rmcp::model::Task::new(
		"task-1".to_string(),
		rmcp::model::TaskStatus::Working,
		"2026-01-01T00:00:00Z".to_string(),
		"2026-01-01T00:00:00Z".to_string(),
	)]);
	a.next_cursor = Some("cursor-a".to_string());
	a.total = Some(1);

	let mut b = ListTasksResult::new(vec![rmcp::model::Task::new(
		"task-2".to_string(),
		rmcp::model::TaskStatus::Working,
		"2026-01-01T00:00:00Z".to_string(),
		"2026-01-01T00:00:00Z".to_string(),
	)]);
	b.next_cursor = Some("cursor-b".to_string());
	b.total = Some(1);

	let merged = relay.merge_tasks(CelExecWrapper::new(Arc::new(None)))(vec![
		(strng::new("serverA"), a.into()),
		(strng::new("serverB"), b.into()),
	])
	.expect("merge should succeed");

	let ServerResult::ListTasksResult(result) = merged else {
		panic!("expected list/tasks result");
	};
	assert_eq!(result.tasks.len(), 2);
	assert_eq!(result.next_cursor, None);
	assert_eq!(result.total, None);
}

#[test]
fn wrap_resource_uri_preserves_template_braces() {
	let router = TargetRouter::new(None);
	let wrapped = router.wrap_resource_uri("counter", "memo://{bucket}/path");
	assert!(wrapped.contains("{bucket}"));
}

#[rstest]
#[case("counter", "memo://insights")]
#[case("server_01", "memo://{bucket}/path")]
#[case("api-prod", "https://example.com/a path?q=a+b&x=1")]
#[case("service9", "urn:uuid:550e8400-e29b-41d4-a716-446655440000")]
#[case("n0de-1", "custom://emoji/\u{2603}")]
fn wrap_resource_uri_roundtrip(#[case] target: &str, #[case] uri: &str) {
	let router = TargetRouter::new(None);
	let wrapped = router.wrap_resource_uri(target, uri).into_owned();
	let parsed = url::Url::parse(&wrapped).expect("wrapped uri should be valid");
	assert_eq!(parsed.scheme(), super::routing::AGW_SCHEME);
	let (unwrapped_target, unwrapped_uri) = router
		.unwrap_resource_uri(&wrapped)
		.expect("wrapped uri should unwrap");
	assert_eq!(unwrapped_target, target);
	assert_eq!(unwrapped_uri, uri);
}

#[rstest]
#[case("serverA", "resource")]
#[case("api1", "name_with_underscores")]
#[case("node9", "dash-and.dot")]
fn resource_name_prefixes_when_multiplexing(#[case] target: &str, #[case] name: &str) {
	let router = TargetRouter::new(None);
	let prefixed = router.resource_name(target, Cow::Borrowed(name));
	assert_eq!(prefixed, format!("{target}{TARGET_NAME_DELIMITER}{name}"));
}

#[tokio::test]
async fn decode_upstream_request_id_handles_colons_in_server_name() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "server::with::colons".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "other".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let id = relay
		.encode_upstream_request_id("server::with::colons", &RequestId::Number(1))
		.expect("request id should encode");
	let (decoded_name, decoded_id) = relay
		.decode_upstream_request_id(&id)
		.expect("decode failed");
	assert_eq!(decoded_name, "server::with::colons");
	assert_eq!(decoded_id, RequestId::Number(1));
}

#[tokio::test]
async fn decode_upstream_request_id_roundtrip_with_separator_in_string_id() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "server::with::colons".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "other".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let original = RequestId::String("my::id:with::separators".into());
	let id = relay
		.encode_upstream_request_id("server::with::colons", &original)
		.expect("request id should encode");
	let (decoded_name, decoded_id) = relay
		.decode_upstream_request_id(&id)
		.expect("decode failed");
	assert_eq!(decoded_name, "server::with::colons");
	assert_eq!(decoded_id, original);
}

#[tokio::test]
async fn decode_upstream_request_id_rejects_mismatched_session_binding() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverA".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverB".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");
	let mismatched_relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverA".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverB".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let id = relay
		.encode_upstream_request_id("serverA", &RequestId::Number(1))
		.expect("request id should encode");
	let err = mismatched_relay
		.decode_upstream_request_id(&id)
		.expect_err("mismatched session binding should be rejected");
	assert!(matches!(err, UpstreamError::InvalidRequest(_)));
}

#[tokio::test]
async fn decode_upstream_request_id_rejects_legacy_format() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverA".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverB".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let legacy = RequestId::String("agw::serverA::n:1".into());
	let err = relay
		.decode_upstream_request_id(&legacy)
		.expect_err("legacy format should be rejected");
	assert!(matches!(err, UpstreamError::InvalidRequest(_)));
}

#[tokio::test]
async fn wrap_resource_uri_roundtrip_with_host_unsafe_target_name() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "prod:api".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "other".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let router = TargetRouter::new(None);
	let wrapped = router
		.wrap_resource_uri("prod:api", "memo://insights")
		.into_owned();
	assert!(wrapped.starts_with("agw://prod%3Aapi/"));
	let (target, uri) = relay
		.unwrap_resource_uri(&wrapped)
		.expect("resource uri should unwrap");
	assert_eq!(target, "prod:api");
	assert_eq!(uri, "memo://insights");
}

#[tokio::test]
async fn send_notification_cancelled_should_route_to_single_upstream_and_rewrite_id() {
	use rmcp::model::{CancelledNotification, CancelledNotificationParam, JsonRpcNotification};

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let rewritten_id = relay
		.encode_upstream_request_id("serverA", &RequestId::Number(1))
		.expect("request id should encode");
	let encoded_id = rewritten_id.to_string();
	let notification = ClientNotification::CancelledNotification(CancelledNotification::new(
		CancelledNotificationParam {
			request_id: rewritten_id,
			reason: Some("client cancelled".to_string()),
		},
	));

	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	wait_until_contains(server_a_capture.path(), "\"notifications/cancelled\"").await;
	wait_until_contains(server_a_capture.path(), "\"requestId\":1").await;
	let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
		.await
		.unwrap_or_default();
	assert!(
		!server_a_raw.contains(&encoded_id),
		"serverA saw encoded request id; expected rewritten id. raw={server_a_raw:?}"
	);

	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/cancelled\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_cancelled_mismatched_session_binding_should_be_dropped() {
	use rmcp::model::{CancelledNotification, CancelledNotificationParam, JsonRpcNotification};

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");
	let mismatched_relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let rewritten_id = relay
		.encode_upstream_request_id("serverA", &RequestId::Number(1))
		.expect("request id should encode");
	let notification = ClientNotification::CancelledNotification(CancelledNotification::new(
		CancelledNotificationParam {
			request_id: rewritten_id,
			reason: Some("client cancelled".to_string()),
		},
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = mismatched_relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	assert_not_contains_for(
		server_a_capture.path(),
		"\"notifications/cancelled\"",
		Duration::from_millis(500),
	)
	.await;
	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/cancelled\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_progress_should_route_to_single_upstream_and_rewrite_token() {
	use rmcp::model::{
		JsonRpcNotification, ProgressNotification, ProgressNotificationParam, ProgressToken,
	};

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let rewritten_progress_token = relay
		.encode_upstream_progress_token("serverA", &ProgressToken(RequestId::Number(1)))
		.expect("progress token should encode");
	let encoded_token = rewritten_progress_token.0.to_string();
	let notification = ClientNotification::ProgressNotification(ProgressNotification::new(
		ProgressNotificationParam::new(rewritten_progress_token, 0.5)
			.with_total(1.0)
			.with_message("halfway"),
	));

	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	wait_until_contains(server_a_capture.path(), "\"notifications/progress\"").await;
	wait_until_contains(server_a_capture.path(), "\"progressToken\":1").await;
	let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
		.await
		.unwrap_or_default();
	assert!(
		!server_a_raw.contains(&encoded_token),
		"serverA saw encoded progress token; expected rewritten token. raw={server_a_raw:?}"
	);

	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/progress\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn map_server_message_rewrites_url_elicitation_identifiers() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverA".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
				Arc::new(crate::mcp::router::McpTarget {
					name: "serverB".into(),
					spec: crate::types::agent::McpTargetSpec::Stdio {
						cmd: "true".into(),
						args: vec![],
						env: Default::default(),
					},
					backend: None,
					always_use_prefix: false,
					backend_policies: Default::default(),
				}),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
		jsonrpc: Default::default(),
		id: RequestId::Number(7),
		request: rmcp::model::ServerRequest::CreateElicitationRequest(
			rmcp::model::CreateElicitationRequest::new(
				rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
					meta: None,
					message: "Open a URL".to_string(),
					url: "https://example.com/flow".to_string(),
					elicitation_id: "elicit-1".to_string(),
				},
			),
		),
	});
	let rewritten_request = relay
		.map_server_message("serverA", request_message)
		.expect("server request should rewrite");
	let ServerJsonRpcMessage::Request(req) = rewritten_request else {
		panic!("expected server request");
	};
	let (request_target, request_id) = relay
		.decode_upstream_request_id(&req.id)
		.expect("request id should decode");
	assert_eq!(request_target, "serverA");
	assert_eq!(request_id, RequestId::Number(7));
	let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
		panic!("expected create elicitation request");
	};
	let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams { elicitation_id, .. } =
		create_req.params
	else {
		panic!("expected URL elicitation params");
	};
	let (target, original_id) = relay
		.decode_upstream_elicitation_id(&elicitation_id)
		.expect("elicitation id should decode");
	assert_eq!(target, "serverA");
	assert_eq!(original_id, "elicit-1");

	let completion_message = ServerJsonRpcMessage::Notification(JsonRpcNotification {
		jsonrpc: Default::default(),
		notification: ServerNotification::ElicitationCompletionNotification(
			rmcp::model::ElicitationCompletionNotification::new(
				rmcp::model::ElicitationResponseNotificationParam::new("elicit-2"),
			),
		),
	});
	let rewritten_completion = relay
		.map_server_message("serverA", completion_message)
		.expect("completion notification should rewrite");
	let ServerJsonRpcMessage::Notification(notification) = rewritten_completion else {
		panic!("expected server notification");
	};
	let ServerNotification::ElicitationCompletionNotification(completion) = notification.notification
	else {
		panic!("expected elicitation completion notification");
	};
	let (target, original_id) = relay
		.decode_upstream_elicitation_id(&completion.params.elicitation_id)
		.expect("elicitation completion id should decode");
	assert_eq!(target, "serverA");
	assert_eq!(original_id, "elicit-2");
}

#[tokio::test]
async fn send_notification_elicitation_response_should_route_to_single_upstream_and_rewrite_id() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
		jsonrpc: Default::default(),
		id: RequestId::Number(11),
		request: rmcp::model::ServerRequest::CreateElicitationRequest(
			rmcp::model::CreateElicitationRequest::new(
				rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
					meta: None,
					message: "Open a URL".to_string(),
					url: "https://example.com/flow".to_string(),
					elicitation_id: "elicit-1".to_string(),
				},
			),
		),
	});
	let rewritten_request = relay
		.map_server_message("serverA", request_message)
		.expect("server request should rewrite");
	let ServerJsonRpcMessage::Request(req) = rewritten_request else {
		panic!("expected server request");
	};
	let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
		panic!("expected create elicitation request");
	};
	let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
		elicitation_id: encoded,
		..
	} = create_req.params
	else {
		panic!("expected URL elicitation params");
	};

	let notification = ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
		ELICITATION_RESPONSE_METHOD,
		Some(json!({
			"elicitationId": encoded,
			"action": "accept"
		})),
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	wait_until_contains(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
	)
	.await;
	wait_until_contains(server_a_capture.path(), "\"elicitationId\":\"elicit-1\"").await;
	let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
		.await
		.unwrap_or_default();
	assert!(
		!server_a_raw.contains("agw::"),
		"serverA saw encoded elicitation id; expected rewritten id. raw={server_a_raw:?}"
	);

	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_elicitation_response_untracked_id_should_be_dropped() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let encoded = relay
		.encode_upstream_elicitation_id("serverA", "elicit-untracked")
		.expect("elicitation id should encode");
	let notification = ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
		ELICITATION_RESPONSE_METHOD,
		Some(json!({
			"elicitationId": encoded,
			"action": "accept"
		})),
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	assert_not_contains_for(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_elicitation_response_session_binding_mismatch_should_be_dropped() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");
	let mismatched_relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let encoded = relay
		.encode_upstream_elicitation_id("serverA", "elicit-mismatch")
		.expect("elicitation id should encode");
	let notification = ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
		ELICITATION_RESPONSE_METHOD,
		Some(json!({
			"elicitationId": encoded,
			"action": "accept"
		})),
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = mismatched_relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	assert_not_contains_for(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_elicitation_response_duplicate_should_be_dropped() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
		jsonrpc: Default::default(),
		id: RequestId::Number(17),
		request: rmcp::model::ServerRequest::CreateElicitationRequest(
			rmcp::model::CreateElicitationRequest::new(
				rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
					meta: None,
					message: "Open a URL".to_string(),
					url: "https://example.com/flow".to_string(),
					elicitation_id: "elicit-dup".to_string(),
				},
			),
		),
	});
	let rewritten_request = relay
		.map_server_message("serverA", request_message)
		.expect("server request should rewrite");
	let ServerJsonRpcMessage::Request(req) = rewritten_request else {
		panic!("expected server request");
	};
	let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
		panic!("expected create elicitation request");
	};
	let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
		elicitation_id: encoded,
		..
	} = create_req.params
	else {
		panic!("expected URL elicitation params");
	};

	let send_response = |elicitation_id: &str| JsonRpcNotification {
		jsonrpc: Default::default(),
		notification: ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
			ELICITATION_RESPONSE_METHOD,
			Some(json!({
				"elicitationId": elicitation_id,
				"action": "accept"
			})),
		)),
	};

	let ctx = IncomingRequestContext::empty();
	relay
		.send_notification(send_response(&encoded), ctx.clone())
		.await
		.expect("first response should be accepted");
	relay
		.send_notification(send_response(&encoded), ctx)
		.await
		.expect("duplicate response should be ignored");

	wait_until_contains(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
	)
	.await;
	let server_a_raw = tokio::fs::read_to_string(server_a_capture.path())
		.await
		.unwrap_or_default();
	assert_eq!(
		server_a_raw
			.matches("\"notifications/elicitation/response\"")
			.count(),
		1,
		"duplicate elicitation response should not be forwarded twice"
	);
	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_elicitation_response_post_resume_should_be_dropped() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let server_b_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay")
	.with_session_binding("session-1", Encoder::base64());
	let resumed_relay = Relay::new(
		McpBackendGroup {
			targets: vec![
				capture_target("serverA", server_a_capture.path()),
				capture_target("serverB", server_b_capture.path()),
			],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay")
	.with_session_binding("session-1", Encoder::base64());

	let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
		jsonrpc: Default::default(),
		id: RequestId::Number(19),
		request: rmcp::model::ServerRequest::CreateElicitationRequest(
			rmcp::model::CreateElicitationRequest::new(
				rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
					meta: None,
					message: "Open a URL".to_string(),
					url: "https://example.com/flow".to_string(),
					elicitation_id: "elicit-resume".to_string(),
				},
			),
		),
	});
	let rewritten_request = relay
		.map_server_message("serverA", request_message)
		.expect("server request should rewrite");
	let ServerJsonRpcMessage::Request(req) = rewritten_request else {
		panic!("expected server request");
	};
	let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
		panic!("expected create elicitation request");
	};
	let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
		elicitation_id: encoded,
		..
	} = create_req.params
	else {
		panic!("expected URL elicitation params");
	};

	let notification = ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
		ELICITATION_RESPONSE_METHOD,
		Some(json!({
			"elicitationId": encoded,
			"action": "accept"
		})),
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = resumed_relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	assert_not_contains_for(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
	assert_not_contains_for(
		server_b_capture.path(),
		"\"notifications/elicitation/response\"",
		Duration::from_millis(500),
	)
	.await;
}

#[tokio::test]
async fn send_notification_elicitation_response_single_target_tracked_id_should_route() {
	use rmcp::model::JsonRpcNotification;

	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let server_a_capture = NamedTempFile::new().expect("temp file");
	let relay = Relay::new(
		McpBackendGroup {
			targets: vec![capture_target("serverA", server_a_capture.path())],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect("relay");

	let request_message = ServerJsonRpcMessage::Request(JsonRpcRequest {
		jsonrpc: Default::default(),
		id: RequestId::Number(13),
		request: rmcp::model::ServerRequest::CreateElicitationRequest(
			rmcp::model::CreateElicitationRequest::new(
				rmcp::model::CreateElicitationRequestParams::UrlElicitationParams {
					meta: None,
					message: "Open a URL".to_string(),
					url: "https://example.com/flow".to_string(),
					elicitation_id: "elicit-single-target".to_string(),
				},
			),
		),
	});
	let rewritten_request = relay
		.map_server_message("serverA", request_message)
		.expect("server request should register elicitation");
	let ServerJsonRpcMessage::Request(req) = rewritten_request else {
		panic!("expected server request");
	};
	let rmcp::model::ServerRequest::CreateElicitationRequest(create_req) = req.request else {
		panic!("expected create elicitation request");
	};
	let rmcp::model::CreateElicitationRequestParams::UrlElicitationParams { elicitation_id, .. } =
		create_req.params
	else {
		panic!("expected URL elicitation params");
	};

	let notification = ClientNotification::CustomNotification(rmcp::model::CustomNotification::new(
		ELICITATION_RESPONSE_METHOD,
		Some(json!({
			"elicitationId": elicitation_id,
			"action": "accept"
		})),
	));
	let r = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification,
	};

	let ctx = IncomingRequestContext::empty();
	let result = relay.send_notification(r, ctx).await;
	assert!(result.is_ok());

	wait_until_contains(
		server_a_capture.path(),
		"\"notifications/elicitation/response\"",
	)
	.await;
	wait_until_contains(
		server_a_capture.path(),
		"\"elicitationId\":\"elicit-single-target\"",
	)
	.await;
}

#[tokio::test]
async fn send_fanout_deletion_allow_degraded_true_ignores_cleanup_errors() {
	let capture = NamedTempFile::new().expect("temp file");
	let relay = single_capture_relay(true, capture.path()).expect("relay with allow_degraded=true");
	let ctx = IncomingRequestContext::empty();

	let first = relay.send_fanout_deletion(ctx.clone()).await;
	assert!(first.is_ok(), "first deletion should succeed");

	let second = relay.send_fanout_deletion(ctx).await;
	assert!(
		second.is_ok(),
		"degraded deletion should ignore cleanup errors"
	);
}

#[tokio::test]
async fn send_fanout_deletion_allow_degraded_false_returns_cleanup_error() {
	let capture = NamedTempFile::new().expect("temp file");
	let relay = single_capture_relay(false, capture.path()).expect("relay with allow_degraded=false");
	let ctx = IncomingRequestContext::empty();

	let first = relay.send_fanout_deletion(ctx.clone()).await;
	assert!(first.is_ok(), "first deletion should succeed");

	let second = relay.send_fanout_deletion(ctx).await;
	assert!(
		second.is_err(),
		"strict deletion should return cleanup errors"
	);
}

#[tokio::test]
async fn send_notification_allow_degraded_true_ignores_delivery_errors() {
	let capture = NamedTempFile::new().expect("temp file");
	let relay = single_capture_relay(true, capture.path()).expect("relay with allow_degraded=true");
	let ctx = IncomingRequestContext::empty();

	let deleted = relay.send_fanout_deletion(ctx.clone()).await;
	assert!(deleted.is_ok(), "upstream shutdown should succeed");

	let notification = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification: ClientNotification::InitializedNotification(
			rmcp::model::InitializedNotification {
				method: Default::default(),
				extensions: Default::default(),
			},
		),
	};
	let result = relay.send_notification(notification, ctx).await;
	assert!(
		result.is_ok(),
		"degraded mode should ignore notification delivery errors"
	);
}

#[tokio::test]
async fn send_notification_allow_degraded_false_returns_delivery_error() {
	let capture = NamedTempFile::new().expect("temp file");
	let relay = single_capture_relay(false, capture.path()).expect("relay with allow_degraded=false");
	let ctx = IncomingRequestContext::empty();

	let deleted = relay.send_fanout_deletion(ctx.clone()).await;
	assert!(deleted.is_ok(), "upstream shutdown should succeed");

	let notification = JsonRpcNotification {
		jsonrpc: Default::default(),
		notification: ClientNotification::InitializedNotification(
			rmcp::model::InitializedNotification {
				method: Default::default(),
				extensions: Default::default(),
			},
		),
	};
	let result = relay.send_notification(notification, ctx).await;
	assert!(
		result.is_err(),
		"strict mode should return notification delivery errors"
	);
}

#[test]
fn relay_rejects_backend_name_containing_delimiter() {
	let test = crate::test_helpers::proxymock::setup_proxy_test("{}").expect("setup_proxy_test");
	let err = Relay::new(
		McpBackendGroup {
			targets: vec![Arc::new(crate::mcp::router::McpTarget {
				name: "bad__name".into(),
				spec: crate::types::agent::McpTargetSpec::Stdio {
					cmd: "true".into(),
					args: vec![],
					env: Default::default(),
				},
				backend: None,
				always_use_prefix: false,
				backend_policies: Default::default(),
			})],
			stateful: false,
			allow_degraded: false,
			allow_insecure_multiplex: false,
		},
		McpAuthorizationSet::new(vec![].into()),
		PolicyClient {
			inputs: test.inputs(),
		},
	)
	.expect_err("should reject backend name containing delimiter");
	assert!(
		err.to_string().contains("reserved delimiter"),
		"unexpected error: {err}"
	);
}
