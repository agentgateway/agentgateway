use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ::http::{Method, Request};
use http_body::Frame;
use hyper_util::client::legacy::Client;
use protos::envoy::service::ext_proc::v3::{
	BodySendMode as EnvoyBodySendMode, ProcessingMode, processing_mode, processing_response,
};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tonic::Status;
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::cel::Expression;
use crate::http::ext_proc::proto::header_value_option::HeaderAppendAction;
use crate::http::ext_proc::proto::{
	BodyMutation, CommonResponse, HeaderMutation, HeaderValue, HeaderValueOption, HttpHeaders,
	ProcessingResponse, body_mutation,
};
use crate::http::ext_proc::{ExtProcDynamicMetadata, proto};
use crate::http::{Body, ext_proc};
use crate::llm::custom;
use crate::test_helpers::extprocmock::{
	ExtProcMock, Handler, immediate_response, request_body_response, request_header_response,
	response_body_response, response_header_response,
};
use crate::test_helpers::proxymock::*;
use crate::test_helpers::{MockInstance, ratelimitmock};
use crate::types::agent::{
	BackendTarget, BackendTrafficPolicy, PolicyInheritance, PolicyTarget, SimpleBackendReference,
	Target, TargetedPolicy,
};
use crate::types::discovery::NamespacedHostname;
use crate::*;

// Processing-option decoding and default behavior.
mod processing_option_defaults {
	use super::*;

	#[test]
	fn mode_state_forces_trailers_send_for_full_duplex() {
		let processing_options = ext_proc::ProcessingOptions {
			request_body_mode: ext_proc::BodySendMode::FullDuplexStreamed,
			response_body_mode: ext_proc::BodySendMode::FullDuplexStreamed,
			request_trailer_mode: ext_proc::TrailerSendMode::Skip,
			response_trailer_mode: ext_proc::TrailerSendMode::Skip,
			..Default::default()
		};

		let mode_state = super::super::ModeStateMachine::from(processing_options);

		assert!(matches!(
			mode_state.request_trailer_mode,
			ext_proc::TrailerSendMode::Send
		));
		assert!(matches!(
			mode_state.response_trailer_mode,
			ext_proc::TrailerSendMode::Send
		));
	}

	#[test]
	fn mode_override_to_full_duplex_forces_corresponding_trailers_send() {
		let processing_options = ext_proc::ProcessingOptions {
			request_body_mode: ext_proc::BodySendMode::None,
			response_body_mode: ext_proc::BodySendMode::None,
			request_trailer_mode: ext_proc::TrailerSendMode::Skip,
			response_trailer_mode: ext_proc::TrailerSendMode::Skip,
			..Default::default()
		};
		let mut mode_state = super::super::ModeStateMachine::from(processing_options);

		mode_state.apply_envoy_mode_override(
			super::super::HeaderPhase::Request,
			&ProcessingMode {
				request_body_mode: processing_mode::BodySendMode::FullDuplexStreamed as i32,
				response_body_mode: processing_mode::BodySendMode::FullDuplexStreamed as i32,
				..Default::default()
			},
		);

		assert!(matches!(
			mode_state.request_trailer_mode,
			ext_proc::TrailerSendMode::Send
		));
		assert!(matches!(
			mode_state.response_trailer_mode,
			ext_proc::TrailerSendMode::Send
		));
	}
}

// End-to-end request/response body mode behavior, including buffered and partial-buffered bodies.
mod body_modes {
	use super::*;

	#[tokio::test]
	async fn nop_ext_proc() {
		let mock = body_mock(b"").await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(NopExtProc::default),
			"{}",
		)
		.await;
		let res = send_request(io, Method::POST, "http://lo").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"");
	}

	#[tokio::test]
	async fn nop_ext_proc_body() {
		let mock = body_mock(b"original").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(NopExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request_body(io, Method::GET, "http://lo", b"request").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		// Server returns no body after ext_proc processes it
		assert_eq!(body.as_ref(), b"");
	}

	#[tokio::test]
	async fn body_based_router() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| BBRExtProc::new(false)),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(
			body
				.headers
				.get("x-gateway-model-name")
				.unwrap()
				.to_str()
				.unwrap(),
			"my-model-name"
		);
	}

	#[tokio::test]
	async fn body_based_router_buffer_body() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| BBRExtProc::new(true)),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(
			body
				.headers
				.get("x-gateway-model-name")
				.unwrap()
				.to_str()
				.unwrap(),
			"my-model-name"
		);
	}

	#[tokio::test]
	async fn buffered_request_body_can_be_replaced() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::Replace(b"rewritten-request".to_vec()),
					BufferedBodyMode::Echo,
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"rewritten-request");
		let replacement_len = b"rewritten-request".len().to_string();
		assert_eq!(
			body
				.headers
				.get("content-length")
				.and_then(|v| v.to_str().ok()),
			Some(replacement_len.as_str())
		);
	}

	#[tokio::test]
	async fn buffered_request_body_rejects_mismatched_content_length() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(BufferedBodyReplacementWithoutContentLengthExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 500);
		let body = read_body_raw(res.into_body()).await;
		assert!(
			body
				.as_ref()
				.starts_with(b"ext_proc failed: invalid body mutation:")
		);
	}

	#[tokio::test]
	async fn buffered_request_streamed_response_removes_content_length() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::StreamedReplace(b"short".to_vec()),
					BufferedBodyMode::Echo,
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"short");
		assert!(body.headers.get("content-length").is_none());
	}

	/// Regression test: buffered body mode must NOT send a spurious end-of-stream RequestBody to the
	/// ext_proc server after the real body has been buffered and processed. ext_proc servers that
	/// respond to unexpected body messages with ImmediateResponse (e.g. the vLLM semantic router)
	/// would otherwise return 400 to the client.
	#[tokio::test]
	async fn buffered_request_body_no_spurious_eos() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(BufferedBodyRejectOnSecondCallExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"rewritten-request");
	}

	#[tokio::test]
	async fn buffered_partial_request_body_can_be_replaced() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "bufferedPartial",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::Replace(b"rewritten-request".to_vec()),
					BufferedBodyMode::Echo,
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"rewritten-request");
		assert!(body.headers.get("content-length").is_none());
	}

	#[tokio::test]
	async fn buffered_partial_request_body_only_mutates_buffered_prefix() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "bufferedPartial",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| {
					ModeAwareBodyExtProc::new(
						BufferedBodyMode::ReplaceAny(b"HELLO".to_vec()),
						BufferedBodyMode::Echo,
					)
				}),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"hello world").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"HELLO world");
		assert!(body.headers.get("content-length").is_none());
	}

	#[tokio::test]
	async fn buffered_request_body_can_be_cleared() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(BufferedBodyMode::Clear, BufferedBodyMode::Echo)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert!(body.body.is_empty());
	}

	#[tokio::test]
	async fn buffered_request_body_noop_preserves_original_body() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(BufferedBodyMode::Noop, BufferedBodyMode::Echo)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let body_in = b"request body";
		let res = tokio::time::timeout(
			Duration::from_secs(3),
			send_request_body(io, Method::POST, "http://lo", body_in),
		)
		.await
		.expect("request timed out while waiting for ext_proc body response");
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), body_in);
	}

	#[tokio::test]
	async fn buffered_request_body_common_response_without_body_mutation_preserves_original_body() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| BufferedBodyNoMutationWithHeaderExtProc),
			"{}",
			Some(processing_options),
		)
		.await;

		let body_in = b"{\"model\":\"gpt-4\",\"messages\":[{\"role\":\"user\",\"content\":\"hello\"}]}";
		let res = tokio::time::timeout(
			Duration::from_secs(3),
			send_request_body(io, Method::POST, "http://lo", body_in),
		)
		.await
		.expect("request timed out while waiting for no-op ext_proc body response");
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), body_in);
		assert_eq!(
			body
				.headers
				.get("x-selected-model")
				.expect("x-selected-model")
				.to_str()
				.unwrap(),
			"gpt-4"
		);
	}

	#[tokio::test]
	async fn buffered_response_body_noop_preserves_original_body() {
		let mock = body_mock(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(BufferedBodyMode::Echo, BufferedBodyMode::Noop)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = tokio::time::timeout(
			Duration::from_secs(3),
			send_request(io, Method::GET, "http://lo"),
		)
		.await
		.expect("request timed out while waiting for ext_proc body response");
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"backend-response");
	}

	#[tokio::test]
	async fn buffered_response_body_can_be_replaced() {
		let mock = body_mock_with_content_length(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::Echo,
					BufferedBodyMode::Replace(b"rewritten-response".to_vec()),
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let replacement_len = b"rewritten-response".len().to_string();
		assert_eq!(
			res
				.headers()
				.get(http::header::CONTENT_LENGTH)
				.and_then(|v| v.to_str().ok()),
			Some(replacement_len.as_str())
		);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"rewritten-response");
	}

	#[tokio::test]
	async fn buffered_response_body_rejects_mismatched_content_length() {
		let mock = body_mock_with_content_length(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(BufferedBodyReplacementWithoutContentLengthExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 500);
		let body = read_body_raw(res.into_body()).await;
		assert!(
			body
				.as_ref()
				.starts_with(b"ext_proc failed: invalid body mutation:")
		);
	}

	#[tokio::test]
	async fn buffered_response_streamed_response_removes_content_length() {
		let mock = body_mock_with_content_length(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::Echo,
					BufferedBodyMode::StreamedReplace(b"short".to_vec()),
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert!(res.headers().get(http::header::CONTENT_LENGTH).is_none());
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"short");
	}

	#[tokio::test]
	async fn buffered_partial_response_body_can_be_replaced() {
		let mock = body_mock(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "bufferedPartial",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(
					BufferedBodyMode::Echo,
					BufferedBodyMode::Replace(b"rewritten-response".to_vec()),
				)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert!(res.headers().get(http::header::CONTENT_LENGTH).is_none());
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"rewritten-response");
	}

	#[tokio::test]
	async fn buffered_partial_response_body_only_mutates_buffered_prefix() {
		let mock = body_mock(b"hello world").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "bufferedPartial",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| {
					ModeAwareBodyExtProc::new(
						BufferedBodyMode::Echo,
						BufferedBodyMode::ReplaceAny(b"HELLO".to_vec()),
					)
				}),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert!(res.headers().get(http::header::CONTENT_LENGTH).is_none());
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"HELLO world");
	}

	#[tokio::test]
	async fn request_body_mode_none_preserves_body_and_content_length() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(NopExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let body_in = b"request body";
		let res = send_request_body(io, Method::POST, "http://lo", body_in).await;
		assert_eq!(res.status(), 200);
		let dump = read_body(res.into_body()).await;
		assert_eq!(dump.body.as_ref(), body_in);
		assert_eq!(
			dump
				.headers
				.get("content-length")
				.and_then(|v| v.to_str().ok()),
			Some(body_in.len().to_string().as_str())
		);
	}

	#[tokio::test]
	async fn response_body_mode_none_preserves_body_and_content_length() {
		let mock = body_mock_with_content_length(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(NopExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert_eq!(
			res
				.headers()
				.get(http::header::CONTENT_LENGTH)
				.and_then(|v| v.to_str().ok()),
			Some("16")
		);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"backend-response");
	}

	#[tokio::test]
	async fn buffered_response_body_can_be_cleared() {
		let mock = body_mock(b"backend-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| {
				ModeAwareBodyExtProc::new(BufferedBodyMode::Echo, BufferedBodyMode::Clear)
			}),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert!(body.is_empty());
	}
}

// Header/trailer phase selection, skipped phases, and first-message metadata/protocol_config.
mod phase_selection {
	use super::*;

	#[tokio::test]
	async fn processing_options_request_header_skip_suppresses_request_headers_message() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "skip",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		assert!(
			captured.iter().all(|r| {
				!matches!(
					r.request,
					Some(proto::processing_request::Request::RequestHeaders(_))
				)
			}),
			"request headers should not be sent when requestHeaderMode=skip"
		);
	}

	#[tokio::test]
	async fn processing_options_response_header_skip_suppresses_response_headers_message() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "skip",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		assert!(
			captured.iter().all(|r| {
				!matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseHeaders(_))
				)
			}),
			"response headers should not be sent when responseHeaderMode=skip"
		);
	}

	#[tokio::test]
	async fn request_header_skip_buffered_sends_attributes_and_protocol_once() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "skip",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta_and_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			None,
			Some(
				[(
					"method".to_string(),
					Arc::new(Expression::new_strict("request.method").unwrap()),
				)]
				.into(),
			),
			None,
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let request_body_messages: Vec<_> = captured
			.iter()
			.filter(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::RequestBody(_))
				)
			})
			.collect();
		assert!(
			!request_body_messages.is_empty(),
			"expected at least one RequestBody message"
		);

		let first = request_body_messages[0];
		let ns_attrs = first
			.attributes
			.get("envoy.filters.http.ext_proc")
			.expect("first RequestBody should include request attributes");
		match &ns_attrs.fields.get("method").unwrap().kind {
			Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "POST"),
			invalid => panic!("expected method string in first RequestBody, got {invalid:?}"),
		}
		let first_proto = first
			.protocol_config
			.as_ref()
			.expect("first RequestBody should include protocol_config");
		assert_eq!(
			first_proto.request_body_mode,
			EnvoyBodySendMode::Buffered as i32
		);

		for msg in request_body_messages.iter().skip(1) {
			assert!(msg.attributes.is_empty());
			assert!(msg.protocol_config.is_none());
		}
	}

	#[tokio::test]
	async fn response_header_skip_buffered_sends_response_attributes_on_first_body_message() {
		let mock = body_mock(b"backend-response").await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "skip",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta_and_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			None,
			None,
			Some(
				[(
					"status".to_string(),
					Arc::new(Expression::new_strict("response.code").unwrap()),
				)]
				.into(),
			),
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let response_body_messages: Vec<_> = captured
			.iter()
			.filter(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseBody(_))
				)
			})
			.collect();
		assert!(
			!response_body_messages.is_empty(),
			"expected at least one ResponseBody message"
		);

		let first = response_body_messages[0];
		let ns_attrs = first
			.attributes
			.get("envoy.filters.http.ext_proc")
			.expect("first ResponseBody should include response attributes");
		match &ns_attrs.fields.get("status").unwrap().kind {
			Some(prost_wkt_types::value::Kind::NumberValue(n)) => assert_eq!(*n, 200.0),
			invalid => panic!("expected status number in first ResponseBody, got {invalid:?}"),
		}

		for msg in response_body_messages.iter().skip(1) {
			assert!(msg.attributes.is_empty());
		}
	}

	#[tokio::test]
	async fn response_headers_first_message_includes_protocol_config() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "skip",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let first = captured
			.first()
			.expect("expected at least one processing request");
		assert!(
			matches!(
				first.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			),
			"expected first processing request to be ResponseHeaders"
		);
		assert!(
			first.protocol_config.is_some(),
			"first processing request should carry protocol_config"
		);

		for msg in captured.iter().skip(1) {
			assert!(msg.protocol_config.is_none());
		}
	}
}

// Mode override behavior for phase transitions that happen from header responses.
mod mode_override_phase_changes {
	use super::*;

	#[tokio::test]
	async fn mode_override_on_request_headers_can_disable_response_headers_phase() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			response_header_mode: processing_mode::HeaderSendMode::Skip as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let saw_response_headers = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			)
		});
		assert!(
			!saw_response_headers,
			"mode_override on request headers should suppress response headers phase"
		);
	}

	#[tokio::test]
	async fn mode_override_on_request_headers_can_disable_buffered_request_body_phase() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			request_body_mode: processing_mode::BodySendMode::None as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let body_in = b"request body";
		let res = send_request_body(io, Method::POST, "http://lo", body_in).await;
		assert_eq!(res.status(), 200);
		let dump = read_body(res.into_body()).await;
		assert_eq!(dump.body.as_ref(), body_in);

		let captured = requests.lock().unwrap();
		let saw_request_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::RequestBody(_))
			)
		});
		assert!(
			!saw_request_body,
			"mode_override on request headers should suppress the request body phase"
		);
	}

	#[tokio::test]
	async fn mode_override_on_request_headers_can_switch_buffered_to_buffered_partial() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| ModeOverrideRequestBodyBufferedPartialExtProc),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"hello world").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"HELLO world");
	}

	#[tokio::test]
	async fn mode_override_on_request_headers_can_enable_buffered_partial_request_body_phase() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| ModeOverrideRequestBodyBufferedPartialExtProc),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"hello world").await;
		assert_eq!(res.status(), 200);
		let body = read_body(res.into_body()).await;
		assert_eq!(body.body.as_ref(), b"HELLO world");
	}

	#[tokio::test]
	async fn mode_override_is_ignored_when_allow_mode_override_is_disabled() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			response_header_mode: processing_mode::HeaderSendMode::Skip as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip"
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let saw_response_headers = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			)
		});
		assert!(
			saw_response_headers,
			"mode_override must be ignored unless allow_mode_override is enabled"
		);
	}

	#[tokio::test]
	async fn mode_override_is_ignored_after_full_duplex_body_streaming_starts() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			response_header_mode: processing_mode::HeaderSendMode::Skip as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"abc").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let request_body_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::RequestBody(_))
				)
			})
			.expect("request body should still be streamed before the request phase completes");
		let response_headers_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseHeaders(_))
				)
			})
			.expect("response headers should still be processed when mode_override is ignored");
		assert!(
			request_body_pos < response_headers_pos,
			"FULL_DUPLEX_STREAMED should preserve the original request-body-first ordering"
		);
		let saw_response_headers = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			)
		});
		assert!(
			saw_response_headers,
			"mode_override must be ignored after full-duplex request body streaming starts"
		);
	}

	#[tokio::test]
	async fn mode_override_on_body_response_is_ignored() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_body_override(ProcessingMode {
			response_header_mode: processing_mode::HeaderSendMode::Skip as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"abc").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let saw_response_headers = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseHeaders(_))
			)
		});
		assert!(
			saw_response_headers,
			"mode_override attached to body response must be ignored"
		);
	}

	#[tokio::test]
	async fn mode_override_on_response_headers_can_disable_response_body_phase() {
		let mock = body_mock(b"upstream-response").await;
		let tracker = ModeOverrideTracker::new().with_response_headers_override(ProcessingMode {
			response_body_mode: processing_mode::BodySendMode::None as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"upstream-response");

		let captured = requests.lock().unwrap();
		let saw_response_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseBody(_))
			)
		});
		assert!(
			!saw_response_body,
			"mode_override on response headers should be able to suppress the response body phase"
		);
	}

	#[tokio::test]
	async fn mode_override_on_response_headers_can_switch_buffered_to_buffered_partial() {
		let mock = body_mock(b"hello world").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| ModeOverrideResponseBodyBufferedPartialExtProc),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"HELLO world");
	}

	#[tokio::test]
	async fn mode_override_on_response_headers_can_enable_buffered_partial_response_body_phase() {
		let mock = body_mock_with_content_length(b"hello world").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) =
			setup_ext_proc_mock_with_processing_options_and_frontend_policy(
				mock,
				ext_proc::FailureMode::FailClosed,
				ExtProcMock::new(|| ModeOverrideResponseBodyBufferedPartialExtProc),
				"{}",
				Some(processing_options),
				json!({
					"http": {
						"maxBufferSize": 5,
					}
				}),
			)
			.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert!(res.headers().get(http::header::CONTENT_LENGTH).is_none());
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"HELLO world");
	}
}

// Immediate responses and fail-open/fail-closed behavior.
mod immediate_and_failure {
	use super::*;

	#[tokio::test]
	async fn immediate_response_request() {
		let mock = simple_mock().await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(ImmediateResponseExtProc::default),
			"{}",
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 202);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"immediate");
	}

	// An ImmediateResponse sent during the request body phase (after a headers `Continue`) must
	// be returned to the client instead of the upstream response.  The body channel is dropped so
	// the upstream may see an empty/truncated body, but the client receives the ext_proc response.
	#[tokio::test]
	async fn immediate_response_request_body_short_circuits() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(ImmediateResponseRequestBodyExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 403);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"Access denied");
	}

	// Regression guard: a normal full-duplex request body (no ImmediateResponse) must stream
	// through unchanged and reach the upstream.
	#[tokio::test]
	async fn full_duplex_request_body_passthrough_not_aborted() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| BBRExtProc::new(false)),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 200);
		let dumped = read_body(res.into_body()).await;
		assert_eq!(dumped.body.as_ref(), b"request");
		let upstream_requests = mock.received_requests().await.unwrap_or_default();
		assert_eq!(
			upstream_requests.len(),
			1,
			"upstream should be contacted exactly once"
		);
	}

	#[tokio::test]
	async fn immediate_response_response() {
		let mock = simple_mock().await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(ImmediateResponseExtProcResponse::default),
			"{}",
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 202);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"immediate");
	}

	#[tokio::test]
	async fn immediate_response_response_body_continuation_does_not_hang() {
		let mock = body_mock(b"upstream-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(ImmediateResponseResponseBodyContinuationExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = tokio::time::timeout(
			Duration::from_secs(3),
			send_request(io, Method::GET, "http://lo"),
		)
		.await
		.expect("response-body continuation should not hang on ImmediateResponse");
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"upstream-response");
	}

	#[tokio::test]
	async fn immediate_response_response_body_phase_returns_direct_response() {
		let mock = body_mock(b"upstream-response").await;
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "buffered",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(ImmediateResponseResponseBodyBufferedFallbackExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;
		let res = send_request(io, Method::GET, "http://lo").await;
		// ImmediateResponse during body phase in the main loop is honored as a
		// DirectResponse because the response has not yet been committed to the
		// downstream client.
		assert_eq!(res.status(), 400);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"late immediate");
	}

	#[tokio::test]
	async fn failure_fail_closed() {
		let mock = simple_mock().await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(FailureExtProcResponse::default),
			"{}",
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 500);
		let body = read_body_raw(res.into_body()).await;
		assert!(body.as_ref().starts_with(b"ext_proc failed:"));
	}

	#[tokio::test]
	async fn failure_fail_open_body() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "send",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailOpen,
			ExtProcMock::new(FailureExtProcResponse::default),
			"{}",
			Some(processing_options),
		)
		.await;

		// If we have a body in FullDuplexStreamed mode, fail open is suppressed
		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 500);
	}

	#[tokio::test]
	async fn failure_fail_open_buffered_request_body_preserves_original_body() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailOpen,
			ExtProcMock::new(FailureExtProcResponse::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let body_in = b"request";
		let res = send_request_body(io, Method::POST, "http://lo", body_in).await;
		assert_eq!(res.status(), 200);
		let dump = read_body(res.into_body()).await;
		assert_eq!(dump.body.as_ref(), body_in);
		assert_eq!(
			dump
				.headers
				.get("content-length")
				.and_then(|v| v.to_str().ok()),
			Some(body_in.len().to_string().as_str())
		);
	}

	#[tokio::test]
	async fn failure_fail_open_suppressed_after_buffered_request_body_starts() {
		let mock = simple_mock().await;
		let processing_options = json!({
			"requestBodyMode": "buffered",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
		});
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailOpen,
			ExtProcMock::new(RequestBodyFailureExtProc::default),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"request").await;
		assert_eq!(res.status(), 500);
		let body = read_body_raw(res.into_body()).await;
		assert!(body.as_ref().starts_with(b"ext_proc failed:"));
	}

	#[tokio::test]
	async fn failure_fail_open() {
		let mock = simple_mock().await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailOpen,
			ExtProcMock::new(FailureExtProcResponse::default),
			"{}",
		)
		.await;

		let res = send_request(io, Method::POST, "http://lo").await;
		assert_eq!(res.status(), 200);
	}
}

// Dynamic metadata propagation through request/response extensions.
mod dynamic_metadata_flow {
	use super::*;

	#[tokio::test]
	async fn dynamic_metadata() {
		let mock = body_mock(b"").await;
		let (_mock, _ext_proc, mut bind, _io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(DynamicMetadataExtProc::default),
			"{}",
		)
		.await;
		bind
			.attach_route_policy(json!({
				"transformations": {
					"response": {
						"set": {
							"x-extproc-metadata": "extproc.some[0]",
						},
					},
				},
			}))
			.await;
		let io = bind.serve_http(strng::new("bind"));
		let res = send_request(io, Method::POST, "http://lo").await;
		assert_eq!(res.status(), 200);
		assert_eq!(
			res
				.headers()
				.get("x-extproc-metadata")
				.unwrap()
				.to_str()
				.unwrap(),
			"a"
		);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"");
	}

	#[tokio::test]
	async fn response_dynamic_metadata_is_attached_to_response_extensions() {
		use prost_wkt_types::value::Kind;
		use prost_wkt_types::{Struct, Value};

		let metadata = Struct {
			fields: [
				(
					"auth_user".to_string(),
					Value {
						kind: Some(Kind::StringValue("test-user".to_string())),
					},
				),
				(
					"is_admin".to_string(),
					Value {
						kind: Some(Kind::BoolValue(true)),
					},
				),
			]
			.into(),
		};
		let presp = ProcessingResponse {
			response: Some(processing_response::Response::ResponseHeaders(
				proto::HeadersResponse { response: None },
			)),
			dynamic_metadata: Some(metadata),
			..Default::default()
		};
		let mut resp = http::Response::new(Body::empty());
		let (mut tx_chunk, _rx_chunk) = mpsc::channel(1);

		let (headers_done, eos) = super::super::handle_response_for_response_mutation(
			false,
			true,
			false,
			Some(&mut resp),
			&mut tx_chunk,
			presp,
		)
		.await
		.expect("response mutation should succeed");

		assert!(headers_done);
		assert!(!eos);
		let metadata = resp
			.extensions()
			.get::<ExtProcDynamicMetadata>()
			.expect("response dynamic metadata should be attached to response extensions");
		assert_eq!(metadata.0.get("auth_user").unwrap(), "test-user");
		assert_eq!(metadata.0.get("is_admin").unwrap(), true);
	}
}

// Shared proxy setup helpers used by the test groups below.
pub async fn setup_ext_proc_mock<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	setup_ext_proc_mock_with_meta(mock, failure_mode, mock_ext_proc, config, None, None, None).await
}

pub async fn setup_ext_proc_mock_with_processing_options<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
	processing_options: Option<serde_json::Value>,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let mut ext_proc_policy = json!({
		"extProc": {
			"host": ext_proc.address,
			"failureMode": failure_mode,
		},
	});
	if let Some(processing_options) = processing_options {
		ext_proc_policy["extProc"]["processingOptions"] = processing_options;
	}

	let t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()))
		.attach_route_policy_builder(ext_proc_policy)
		.await;
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
}

pub async fn setup_ext_proc_mock_with_processing_options_and_frontend_policy<
	T: Handler + Send + Sync + 'static,
>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
	processing_options: Option<serde_json::Value>,
	frontend_policy: serde_json::Value,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let mut ext_proc_policy = json!({
		"extProc": {
			"host": ext_proc.address,
			"failureMode": failure_mode,
		},
	});
	if let Some(processing_options) = processing_options {
		ext_proc_policy["extProc"]["processingOptions"] = processing_options;
	}

	let mut t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()))
		.attach_route_policy_builder(ext_proc_policy)
		.await;
	t.attach_frontend_policy(frontend_policy).await;
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
}

#[allow(clippy::too_many_arguments)]
pub async fn setup_ext_proc_mock_with_meta_and_processing_options<
	T: Handler + Send + Sync + 'static,
>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
	metadata_context: Option<HashMap<String, HashMap<String, Arc<Expression>>>>,
	request_attributes: Option<HashMap<String, Arc<Expression>>>,
	response_attributes: Option<HashMap<String, Arc<Expression>>>,
	processing_options: Option<serde_json::Value>,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let mut ext_proc_policy = json!({
		"extProc": {
			"host": ext_proc.address,
			"failureMode": failure_mode,
			"metadataContext": metadata_context,
			"requestAttributes": request_attributes,
			"responseAttributes": response_attributes,
		},
	});
	if let Some(processing_options) = processing_options {
		ext_proc_policy["extProc"]["processingOptions"] = processing_options;
	}

	let t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()))
		.attach_route_policy_builder(ext_proc_policy)
		.await;
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
}

pub async fn setup_ext_proc_mock_with_meta<T: Handler + Send + Sync + 'static>(
	mock: MockServer,
	failure_mode: ext_proc::FailureMode,
	mock_ext_proc: ExtProcMock<T>,
	config: &str,
	metadata_context: Option<HashMap<String, HashMap<String, Arc<Expression>>>>,
	request_attributes: Option<HashMap<String, Arc<Expression>>>,
	response_attributes: Option<HashMap<String, Arc<Expression>>>,
) -> (
	MockServer,
	MockInstance,
	TestBind,
	Client<MemoryConnector, Body>,
) {
	let ext_proc = mock_ext_proc.spawn().await;

	let t = setup_proxy_test(config)
		.unwrap()
		.with_backend(*mock.address())
		.with_backend(ext_proc.address)
		.with_bind(simple_bind())
		.with_route(basic_route(*mock.address()))
		.attach_route_policy_builder(json!({
			"extProc": {
				"host": ext_proc.address,
				"failureMode": failure_mode,
				"metadataContext": metadata_context,
				"requestAttributes": request_attributes,
				"responseAttributes": response_attributes,
			}
		}))
		.await;
	let io = t.serve_http(strng::new("bind"));
	(mock, ext_proc, t, io)
}

fn build_ext_proc_request_for_test(
	ext_proc_addr: std::net::SocketAddr,
	processing_options: ext_proc::ProcessingOptions,
) -> super::ExtProcRequest {
	let bind = setup_proxy_test("{}").unwrap().with_backend(ext_proc_addr);
	let client = crate::proxy::httpproxy::PolicyClient::new(bind.inputs());
	super::ExtProc {
		target: Arc::new(crate::types::agent::SimpleBackendReference::Backend(
			strng::format!("/{}", ext_proc_addr),
		)),
		policies: Vec::new(),
		failure_mode: ext_proc::FailureMode::FailClosed,
		metadata_context: None,
		request_attributes: None,
		response_attributes: None,
		processing_options,
	}
	.build(client)
}

const STANDALONE_SERVICE_NAME: &str = "model-service.default.svc.cluster.local";
const STANDALONE_SERVICE_REF: &str = "default/model-service.default.svc.cluster.local";
const STANDALONE_SERVICE_PORT: u16 = 8000;

#[derive(Clone)]
struct StandaloneInferenceRouter {
	target: Option<SocketAddr>,
	request_headers_seen: Arc<AtomicUsize>,
	request_path_seen: Option<Arc<Mutex<Option<String>>>>,
	request_body_seen: Option<Arc<Mutex<Vec<u8>>>>,
}

impl StandaloneInferenceRouter {
	fn new(target: Option<SocketAddr>, request_headers_seen: Arc<AtomicUsize>) -> Self {
		Self {
			target,
			request_headers_seen,
			request_path_seen: None,
			request_body_seen: None,
		}
	}

	fn recording(
		target: Option<SocketAddr>,
		request_headers_seen: Arc<AtomicUsize>,
		request_path_seen: Arc<Mutex<Option<String>>>,
		request_body_seen: Arc<Mutex<Vec<u8>>>,
	) -> Self {
		Self {
			target,
			request_headers_seen,
			request_path_seen: Some(request_path_seen),
			request_body_seen: Some(request_body_seen),
		}
	}
}

#[derive(Clone)]
struct BodyDrivenStandaloneInferenceRouter {
	target: SocketAddr,
	request_bodies_seen: Arc<AtomicUsize>,
	sent_headers: bool,
}

#[async_trait::async_trait]
impl Handler for StandaloneInferenceRouter {
	async fn handle_request_headers(
		&mut self,
		headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.request_headers_seen.fetch_add(1, Ordering::SeqCst);
		if let Some(request_path_seen) = &self.request_path_seen
			&& let Some(path) = headers.headers.as_ref().and_then(|headers| {
				headers
					.headers
					.iter()
					.find(|header| header.key == ":path")
					.map(|header| header.value.clone())
			}) {
			*request_path_seen.lock().unwrap() = Some(path);
		}
		let _ = sender
			.send(request_header_response(self.target.map(|target| {
				CommonResponse {
					header_mutation: Some(HeaderMutation {
						set_headers: vec![HeaderValueOption {
							header: Some(HeaderValue {
								key: "x-gateway-destination-endpoint".to_string(),
								value: target.to_string(),
								raw_value: Vec::new(),
							}),
							append: Some(false),
							..Default::default()
						}],
						remove_headers: vec![],
					}),
					..Default::default()
				}
			})))
			.await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if let Some(request_body_seen) = &self.request_body_seen {
			request_body_seen
				.lock()
				.unwrap()
				.extend_from_slice(&body.body);
		}
		let _ = sender
			.send(request_body_response(Some(CommonResponse {
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::StreamedResponse(
						proto::StreamedBodyResponse {
							body: body.body.clone(),
							end_of_stream: body.end_of_stream,
						},
					)),
				}),
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

#[derive(Clone)]
struct RecordingRateLimit {
	requests: mpsc::UnboundedSender<crate::http::remoteratelimit::proto::RateLimitRequest>,
}

#[async_trait::async_trait]
impl ratelimitmock::Handler for RecordingRateLimit {
	async fn should_rate_limit(
		&mut self,
		request: &crate::http::remoteratelimit::proto::RateLimitRequest,
	) -> Result<crate::http::remoteratelimit::proto::RateLimitResponse, tonic::Status> {
		self
			.requests
			.send(request.clone())
			.expect("rate limit request receiver should be open");
		ratelimitmock::ok_response()
	}
}

async fn recv_rate_limit_request(
	requests: &mut mpsc::UnboundedReceiver<crate::http::remoteratelimit::proto::RateLimitRequest>,
) -> crate::http::remoteratelimit::proto::RateLimitRequest {
	tokio::time::timeout(Duration::from_secs(1), requests.recv())
		.await
		.expect("timed out waiting for rate limit request")
		.expect("rate limit request sender should be open")
}

#[async_trait::async_trait]
impl Handler for BodyDrivenStandaloneInferenceRouter {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		_sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.request_bodies_seen.fetch_add(1, Ordering::SeqCst);
		if !self.sent_headers {
			self.sent_headers = true;
			let _ = sender
				.send(request_header_response(Some(CommonResponse {
					header_mutation: Some(HeaderMutation {
						set_headers: vec![HeaderValueOption {
							header: Some(HeaderValue {
								key: "x-gateway-destination-endpoint".to_string(),
								value: self.target.to_string(),
								raw_value: Vec::new(),
							}),
							append: Some(false),
							..Default::default()
						}],
						remove_headers: vec![],
					}),
					..Default::default()
				})))
				.await;
		}
		let _ = sender
			.send(request_body_response(Some(CommonResponse {
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::StreamedResponse(
						proto::StreamedBodyResponse {
							body: body.body.clone(),
							end_of_stream: body.end_of_stream,
						},
					)),
				}),
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

async fn named_backend(body: &'static str) -> MockServer {
	let mock = MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(ResponseTemplate::new(200).set_body_string(body))
		.mount(&mock)
		.await;
	mock
}

fn configure_standalone_service(t: &TestBind) {
	use crate::types::discovery::{NetworkAddress, Service};

	let service = Service {
		name: "model-service".into(),
		namespace: "default".into(),
		hostname: STANDALONE_SERVICE_NAME.into(),
		vips: vec![NetworkAddress {
			network: strng::EMPTY,
			address: "127.0.0.1".parse().unwrap(),
		}],
		ports: HashMap::from([(STANDALONE_SERVICE_PORT, STANDALONE_SERVICE_PORT)]),
		..Default::default()
	};

	t.pi
		.stores
		.discovery
		.sync_local(vec![service], vec![], Default::default())
		.unwrap();
}

async fn setup_inference_routing_mock(
	target: Option<SocketAddr>,
	request_headers_seen: Arc<AtomicUsize>,
	destination_mode: Option<&'static str>,
) -> (MockInstance, TestBind, Client<MemoryConnector, Body>) {
	let ext_proc =
		ExtProcMock::new(move || StandaloneInferenceRouter::new(target, request_headers_seen.clone()))
			.spawn()
			.await;

	let mut t = setup_proxy_test("{}").unwrap().with_bind(simple_bind());
	configure_standalone_service(&t);
	let mut inference_routing = json!({
		"endpointPicker": {
			"host": ext_proc.address.to_string(),
		},
	});
	if let Some(destination_mode) = destination_mode {
		inference_routing["destinationMode"] = json!(destination_mode);
	}
	t.attach_route(json!({
		"name": "standalone-epp",
		"backends": [
			{
				"service": {
					"name": STANDALONE_SERVICE_REF,
					"port": STANDALONE_SERVICE_PORT,
				},
				"policies": {
					"inferenceRouting": inference_routing,
				},
			}
		],
	}))
	.await;
	let io = t.serve_http(BIND_KEY);
	(ext_proc, t, io)
}

async fn setup_body_driven_inference_routing_mock(
	target: SocketAddr,
	request_bodies_seen: Arc<AtomicUsize>,
) -> (MockInstance, TestBind, Client<MemoryConnector, Body>) {
	let ext_proc = ExtProcMock::new(move || BodyDrivenStandaloneInferenceRouter {
		target,
		request_bodies_seen: request_bodies_seen.clone(),
		sent_headers: false,
	})
	.spawn()
	.await;

	let mut t = setup_proxy_test("{}").unwrap().with_bind(simple_bind());
	configure_standalone_service(&t);
	t.attach_route(json!({
		"name": "standalone-epp-body",
		"backends": [
			{
				"service": {
					"name": STANDALONE_SERVICE_REF,
					"port": STANDALONE_SERVICE_PORT,
				},
				"policies": {
					"inferenceRouting": {
						"endpointPicker": {
							"host": ext_proc.address.to_string(),
						},
						"destinationMode": "passthrough",
					},
				},
			}
		],
	}))
	.await;
	let io = t.serve_http(BIND_KEY);
	(ext_proc, t, io)
}

// Standalone inference routing uses the ext_proc/EPP path to choose or validate destinations.
mod standalone_inference_routing {
	use super::*;

	#[tokio::test]
	async fn standalone_inference_routing_uses_epp_selected_destination_without_local_endpoints() {
		let backend_a = named_backend("backend-a").await;
		let backend_b = named_backend("backend-b").await;
		let request_headers_seen = Arc::new(AtomicUsize::new(0));
		let (_ext_proc, _bind, io) = setup_inference_routing_mock(
			Some(*backend_b.address()),
			request_headers_seen.clone(),
			Some("passthrough"),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let body = read_body_raw(res.into_body()).await;
		assert_eq!(body.as_ref(), b"backend-b");
		assert_eq!(
			request_headers_seen.load(Ordering::SeqCst),
			1,
			"request should consult the local EPP",
		);
		assert_eq!(
			backend_a
				.received_requests()
				.await
				.expect("backend-a recording should be enabled")
				.len(),
			0,
			"non-selected service endpoints should not receive traffic",
		);
		assert_eq!(
			backend_b
				.received_requests()
				.await
				.expect("backend-b recording should be enabled")
				.len(),
			1,
			"EPP-selected endpoint should receive traffic",
		);
	}

	#[tokio::test]
	async fn standalone_inference_routing_streams_body_before_header_response() {
		let backend = named_backend("backend").await;
		let request_bodies_seen = Arc::new(AtomicUsize::new(0));
		let (_ext_proc, _bind, io) =
			setup_body_driven_inference_routing_mock(*backend.address(), request_bodies_seen.clone())
				.await;

		let res = tokio::time::timeout(
			Duration::from_secs(3),
			send_request_body(io, Method::POST, "http://lo", b"request body"),
		)
		.await
		.expect("inference routing timed out waiting for body-driven EPP response");
		assert_eq!(res.status(), 200);
		assert!(
			request_bodies_seen.load(Ordering::SeqCst) > 0,
			"inference routing should stream the request body before receiving a header response",
		);
	}

	#[tokio::test]
	async fn standalone_inference_routing_validates_selected_destination_by_default() {
		let backend = named_backend("backend").await;
		let request_headers_seen = Arc::new(AtomicUsize::new(0));
		let (_ext_proc, _bind, io) =
			setup_inference_routing_mock(Some(*backend.address()), request_headers_seen.clone(), None)
				.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 503);
		assert_eq!(
			request_headers_seen.load(Ordering::SeqCst),
			1,
			"gateway should consult the local EPP",
		);
		assert_eq!(
			backend
				.received_requests()
				.await
				.expect("backend recording should be enabled")
				.len(),
			0,
			"validated mode should reject destinations outside local service endpoints",
		);
	}

	#[tokio::test]
	async fn standalone_inference_routing_requires_epp_selected_destination() {
		let request_headers_seen = Arc::new(AtomicUsize::new(0));
		let (_ext_proc, _bind, io) =
			setup_inference_routing_mock(None, request_headers_seen.clone(), Some("passthrough")).await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 503);
		assert_eq!(
			request_headers_seen.load(Ordering::SeqCst),
			1,
			"gateway should consult EPP before rejecting the request",
		);
	}
}

#[tokio::test]
async fn custom_llm_provider_service_backend_runs_inference_routing() {
	let backend = body_mock(include_bytes!(
		"../llm/tests/response/completions/basic.json"
	))
	.await;
	let backend_addr = *backend.address();
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let ext_proc = ExtProcMock::new({
		let request_headers_seen = request_headers_seen.clone();
		move || StandaloneInferenceRouter::new(Some(backend_addr), request_headers_seen.clone())
	})
	.spawn()
	.await;

	let backend_name = "custom-ai";
	let service = NamespacedHostname {
		namespace: "default".into(),
		hostname: STANDALONE_SERVICE_NAME.into(),
	};
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(simple_bind())
		.with_raw_backend(custom_llm_backend(
			backend_name,
			SimpleBackendReference::Service {
				name: service.clone(),
				port: STANDALONE_SERVICE_PORT,
			},
			vec![custom::ProviderFormat::Completions],
		))
		.with_route(basic_named_route(strng::format!("/{backend_name}")));
	configure_standalone_service(&t);
	t.with_policy(TargetedPolicy {
		key: "custom-provider-epp".into(),
		name: None,
		inheritance: PolicyInheritance::default(),
		target: PolicyTarget::Backend(BackendTarget::Service {
			hostname: service.hostname.clone(),
			namespace: service.namespace.clone(),
			port: Some(STANDALONE_SERVICE_PORT),
		}),
		policy: BackendTrafficPolicy::InferenceRouting(ext_proc::InferenceRouting {
			target: Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
				ext_proc.address,
			))),
			destination_mode: ext_proc::InferenceRoutingDestinationMode::Passthrough,
			failure_mode: ext_proc::FailureMode::FailClosed,
		})
		.into(),
	});
	let io = t.serve_http(BIND_KEY);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let _ = read_body_raw(res.into_body()).await;
	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		1,
		"provider service backend should consult EPP",
	);
	assert_eq!(
		backend
			.received_requests()
			.await
			.expect("backend recording should be enabled")
			.len(),
		1,
		"EPP-selected backend should receive the LLM request",
	);
}

#[tokio::test]
async fn custom_llm_provider_inference_routing_sees_input_shape_and_amends_token_rate_limit() {
	let backend = body_mock(include_bytes!("../llm/tests/response/anthropic/basic.json")).await;
	let backend_addr = *backend.address();
	let request_headers_seen = Arc::new(AtomicUsize::new(0));
	let request_path_seen = Arc::new(Mutex::new(None));
	let request_body_seen = Arc::new(Mutex::new(Vec::new()));
	let ext_proc = ExtProcMock::new({
		let request_headers_seen = request_headers_seen.clone();
		let request_path_seen = request_path_seen.clone();
		let request_body_seen = request_body_seen.clone();
		move || {
			StandaloneInferenceRouter::recording(
				Some(backend_addr),
				request_headers_seen.clone(),
				request_path_seen.clone(),
				request_body_seen.clone(),
			)
		}
	})
	.spawn()
	.await;

	let (rate_limit_tx, mut rate_limit_rx) = mpsc::unbounded_channel();
	let rate_limit = ratelimitmock::RateLimitMock::new({
		let rate_limit_tx = rate_limit_tx.clone();
		move || RecordingRateLimit {
			requests: rate_limit_tx.clone(),
		}
	})
	.spawn()
	.await;

	let backend_name = "custom-ai";
	let service = NamespacedHostname {
		namespace: "default".into(),
		hostname: STANDALONE_SERVICE_NAME.into(),
	};
	let mut t = setup_proxy_test("{}")
		.unwrap()
		.with_bind(simple_bind())
		.with_raw_backend(custom_llm_backend(
			backend_name,
			SimpleBackendReference::Service {
				name: service.clone(),
				port: STANDALONE_SERVICE_PORT,
			},
			vec![custom::ProviderFormat::Messages],
		))
		.with_route(basic_named_route(strng::format!("/{backend_name}")));
	configure_standalone_service(&t);
	t.with_policy(TargetedPolicy {
		key: "custom-provider-epp".into(),
		name: None,
		inheritance: PolicyInheritance::default(),
		target: PolicyTarget::Backend(BackendTarget::Service {
			hostname: service.hostname.clone(),
			namespace: service.namespace.clone(),
			port: Some(STANDALONE_SERVICE_PORT),
		}),
		policy: BackendTrafficPolicy::InferenceRouting(ext_proc::InferenceRouting {
			target: Arc::new(SimpleBackendReference::InlineBackend(Target::Address(
				ext_proc.address,
			))),
			destination_mode: ext_proc::InferenceRoutingDestinationMode::Passthrough,
			failure_mode: ext_proc::FailureMode::FailClosed,
		})
		.into(),
	});
	t.attach_route_policy(json!({
		"remoteRateLimit": {
			"domain": "llm",
			"host": rate_limit.address.to_string(),
			"descriptors": [{
				"entries": [{
					"key": "model",
					"value": "\"model\"",
				}],
				"type": "tokens",
				"cost": "llm.outputTokens * uint(1000) + llm.inputTokens",
			}],
		},
	}))
	.await;
	let io = t.serve_http(BIND_KEY);

	let res = send_request_body(
		io,
		Method::POST,
		"http://lo/v1/chat/completions",
		include_bytes!("../llm/tests/requests/completions/basic.json"),
	)
	.await;
	assert_eq!(res.status(), 200);
	let response_body: serde_json::Value =
		serde_json::from_slice(&read_body_raw(res.into_body()).await).expect("response is JSON");
	assert_eq!(response_body["object"], "chat.completion");

	assert_eq!(
		request_headers_seen.load(Ordering::SeqCst),
		1,
		"provider service backend should consult EPP",
	);
	assert_eq!(
		request_path_seen.lock().unwrap().as_deref(),
		Some("/v1/chat/completions"),
		"EPP should see the client request path before upstream serialization",
	);
	let epp_body: serde_json::Value =
		serde_json::from_slice(&request_body_seen.lock().unwrap()).expect("EPP body is JSON");
	assert_eq!(epp_body["messages"][0]["role"], "system");
	assert!(epp_body.get("system").is_none());

	let requests = backend
		.received_requests()
		.await
		.expect("backend recording should be enabled");
	assert_eq!(requests.len(), 1);
	assert_eq!(requests[0].url.path(), "/v1/messages");
	let upstream_body: serde_json::Value =
		serde_json::from_slice(&requests[0].body).expect("upstream request should be JSON");
	assert_eq!(upstream_body["system"], "You are a helpful assistant.");
	assert_eq!(upstream_body["messages"][0]["role"], "user");

	let initial_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	let amend_request = recv_rate_limit_request(&mut rate_limit_rx).await;
	assert_eq!(initial_request.domain, "llm");
	assert_eq!(amend_request.domain, "llm");
	assert_eq!(
		initial_request.descriptors.first().unwrap().hits_addend,
		Some(0)
	);
	assert_eq!(
		amend_request.descriptors.first().unwrap().hits_addend,
		Some(21015)
	);
}

// Shared ext_proc mock handlers used by the end-to-end tests above and below.
#[derive(Debug, Default)]
struct NopExtProc {
	sent_req_body: bool,
	sent_resp_body: bool,
}

#[async_trait::async_trait]
impl Handler for NopExtProc {
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_resp_body {
			let _ = sender.send(response_body_response(None)).await;
		}
		self.sent_resp_body = true;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct DynamicMetadataExtProc {
	sent_req_body: bool,
	sent_resp_body: bool,
}

#[async_trait::async_trait]
impl Handler for DynamicMetadataExtProc {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		use prost_wkt_types::Value;
		use prost_wkt_types::value::Kind;

		let _ = sender
			.send(Ok(ProcessingResponse {
				response: Some(processing_response::Response::RequestHeaders(
					proto::HeadersResponse { response: None },
				)),
				dynamic_metadata: Some(prost_wkt_types::Struct {
					fields: HashMap::from([(
						"some".to_string(),
						Value {
							kind: Some(Kind::ListValue(prost_wkt_types::ListValue {
								values: vec![
									Value {
										kind: Some(Kind::StringValue("a".to_string())),
									},
									Value {
										kind: Some(Kind::StringValue("b".to_string())),
									},
								],
							})),
						},
					)]),
				}),
				..Default::default()
			}))
			.await;
		Ok(())
	}
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_resp_body {
			let _ = sender.send(response_body_response(None)).await;
		}
		self.sent_resp_body = true;
		Ok(())
	}
}

/// Simulate GIE body based router
#[derive(Debug)]
struct BBRExtProc {
	req_body: Vec<u8>,
	buffer_body: bool,
	res_body: Vec<u8>,
}

#[derive(Clone, Debug)]
enum BufferedBodyMode {
	Echo,
	Replace(Vec<u8>),
	ReplaceAny(Vec<u8>),
	StreamedReplace(Vec<u8>),
	Clear,
	Noop,
}

#[derive(Debug)]
struct ModeAwareBodyExtProc {
	request_body_mode: BufferedBodyMode,
	response_body_mode: BufferedBodyMode,
}

impl ModeAwareBodyExtProc {
	fn new(request_body_mode: BufferedBodyMode, response_body_mode: BufferedBodyMode) -> Self {
		Self {
			request_body_mode,
			response_body_mode,
		}
	}

	fn content_length_header_mutation(len: usize) -> HeaderMutation {
		HeaderMutation {
			set_headers: vec![HeaderValueOption {
				header: Some(HeaderValue {
					key: "content-length".to_string(),
					value: String::new(),
					raw_value: len.to_string().into_bytes(),
				}),
				append: Some(false),
				..Default::default()
			}],
			remove_headers: Vec::new(),
		}
	}

	fn body_response(mode: &BufferedBodyMode, body: &proto::HttpBody) -> Option<CommonResponse> {
		match mode {
			BufferedBodyMode::Echo => Some(CommonResponse {
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::StreamedResponse(
						proto::StreamedBodyResponse {
							body: body.body.clone(),
							end_of_stream: body.end_of_stream,
						},
					)),
				}),
				..Default::default()
			}),
			BufferedBodyMode::Replace(replacement) if body.end_of_stream => Some(CommonResponse {
				header_mutation: Some(Self::content_length_header_mutation(replacement.len())),
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::Body(replacement.clone().into())),
				}),
				..Default::default()
			}),
			BufferedBodyMode::ReplaceAny(replacement) => Some(CommonResponse {
				header_mutation: Some(Self::content_length_header_mutation(replacement.len())),
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::Body(replacement.clone().into())),
				}),
				..Default::default()
			}),
			BufferedBodyMode::StreamedReplace(replacement) => Some(CommonResponse {
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::StreamedResponse(
						proto::StreamedBodyResponse {
							body: replacement.clone().into(),
							end_of_stream: body.end_of_stream,
						},
					)),
				}),
				..Default::default()
			}),
			BufferedBodyMode::Clear if body.end_of_stream => Some(CommonResponse {
				header_mutation: Some(Self::content_length_header_mutation(0)),
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::ClearBody(true)),
				}),
				..Default::default()
			}),
			BufferedBodyMode::Noop => None,
			_ => None,
		}
	}
}

#[async_trait::async_trait]
impl Handler for ModeAwareBodyExtProc {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		match &self.request_body_mode {
			BufferedBodyMode::Noop => {
				let _ = sender.send(request_body_response(None)).await;
			},
			_ => {
				let response = Self::body_response(&self.request_body_mode, body);
				if let Some(response) = response {
					let _ = sender.send(request_body_response(Some(response))).await;
				}
			},
		}
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		match &self.response_body_mode {
			BufferedBodyMode::Noop => {
				let _ = sender.send(response_body_response(None)).await;
			},
			_ => {
				let response = Self::body_response(&self.response_body_mode, body);
				if let Some(response) = response {
					let _ = sender.send(response_body_response(Some(response))).await;
				}
			},
		}
		Ok(())
	}
}

impl BBRExtProc {
	pub fn new(buffer_body: bool) -> Self {
		Self {
			buffer_body,
			req_body: Default::default(),
			res_body: Default::default(),
		}
	}
}

// https://github.com/kubernetes-sigs/gateway-api-inference-extension/blob/2a187ea174ed2fafd22e6aff8cb13e532dc7604e/pkg/bbr/handlers/server.go#L74
#[async_trait::async_trait]
impl Handler for BBRExtProc {
	async fn handle_request_headers(
		&mut self,
		headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if headers.end_of_stream {
			let _ = sender.send(request_header_response(None)).await;
		}
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.req_body.extend_from_slice(&body.body);
		if body.end_of_stream {
			let _ = sender
				.send(request_header_response(Some(CommonResponse {
					header_mutation: Some(HeaderMutation {
						set_headers: vec![HeaderValueOption {
							header: Some(HeaderValue {
								key: "X-Gateway-Model-Name".to_string(),
								value: String::new(),
								raw_value: b"my-model-name".to_vec(),
							}),
							append: None,
							append_action: 0,
						}],
						remove_headers: vec![],
					}),
					..Default::default()
				})))
				.await;
			let _ = sender
				.send(request_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: self.req_body.clone().into(),
								end_of_stream: true,
							},
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if self.buffer_body {
			self.res_body.extend_from_slice(&body.body);
			if body.end_of_stream {
				let _ = sender
					.send(response_body_response(Some(CommonResponse {
						body_mutation: Some(BodyMutation {
							mutation: Some(body_mutation::Mutation::StreamedResponse(
								proto::StreamedBodyResponse {
									body: self.res_body.clone().into(),
									end_of_stream: true,
								},
							)),
						}),
						..Default::default()
					})))
					.await;
			}
		} else {
			let _ = sender
				.send(response_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::StreamedResponse(
							proto::StreamedBodyResponse {
								body: body.body.clone(),
								end_of_stream: body.end_of_stream,
							},
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseExtProc {}

#[async_trait::async_trait]
impl Handler for ImmediateResponseExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 202 }),
				body: "immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseRequestBodyExtProc {
	sent: bool,
}

#[async_trait::async_trait]
impl Handler for ImmediateResponseRequestBodyExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent {
			self.sent = true;
			let _ = sender
				.send(immediate_response(proto::ImmediateResponse {
					status: Some(proto::HttpStatus {
						code: proto::StatusCode::Forbidden as i32,
					}),
					body: "Access denied".to_string(),
					headers: None,
					grpc_status: None,
					details: "".to_string(),
				}))
				.await;
		}
		Ok(())
	}
}

#[derive(Debug)]
struct BufferedBodyNoMutationWithHeaderExtProc;

#[async_trait::async_trait]
impl Handler for BufferedBodyNoMutationWithHeaderExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(request_body_response(Some(CommonResponse {
				header_mutation: Some(HeaderMutation {
					set_headers: vec![HeaderValueOption {
						header: Some(HeaderValue {
							key: "x-selected-model".to_string(),
							value: String::new(),
							raw_value: b"gpt-4".to_vec(),
						}),
						append: Some(false),
						..Default::default()
					}],
					remove_headers: Vec::new(),
				}),
				body_mutation: None,
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

/// Handler for `buffered_request_body_no_spurious_eos`. On the first body call it replaces
/// the body; on any subsequent call it sends ImmediateResponse(400) to simulate ext_proc servers
/// (e.g. vLLM semantic router) that reject unexpected body messages.
#[derive(Debug, Default)]
struct BufferedBodyRejectOnSecondCallExtProc {
	calls: usize,
}

#[async_trait::async_trait]
impl Handler for BufferedBodyRejectOnSecondCallExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		self.calls += 1;
		if self.calls == 1 {
			let _ = sender
				.send(request_body_response(Some(CommonResponse {
					header_mutation: Some(ModeAwareBodyExtProc::content_length_header_mutation(
						b"rewritten-request".len(),
					)),
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::Body(
							b"rewritten-request".to_vec().into(),
						)),
					}),
					..Default::default()
				})))
				.await;
		} else {
			let _ = sender
				.send(immediate_response(proto::ImmediateResponse {
					status: Some(proto::HttpStatus { code: 400 }),
					body: "malformed JSON body".to_string(),
					headers: None,
					grpc_status: None,
					details: "".to_string(),
				}))
				.await;
		}
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseExtProcResponse {
	sent_req_body: bool,
}

#[async_trait::async_trait]
impl Handler for ImmediateResponseExtProcResponse {
	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if !self.sent_req_body {
			let _ = sender.send(request_body_response(None)).await;
		}
		self.sent_req_body = true;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 202 }),
				body: "immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseResponseBodyContinuationExtProc;

#[async_trait::async_trait]
impl Handler for ImmediateResponseResponseBodyContinuationExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(response_body_response(Some(CommonResponse {
				body_mutation: Some(BodyMutation {
					mutation: Some(body_mutation::Mutation::StreamedResponse(
						proto::StreamedBodyResponse {
							body: body.body.clone(),
							end_of_stream: false,
						},
					)),
				}),
				..Default::default()
			})))
			.await;
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 400 }),
				body: "late immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct ImmediateResponseResponseBodyBufferedFallbackExtProc;

#[async_trait::async_trait]
impl Handler for ImmediateResponseResponseBodyBufferedFallbackExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_header_response(None)).await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(immediate_response(proto::ImmediateResponse {
				status: Some(proto::HttpStatus { code: 400 }),
				body: "late immediate".to_string(),
				headers: None,
				grpc_status: None,
				details: "".to_string(),
			}))
			.await;
		Ok(())
	}
}

#[derive(Debug, Default)]
struct FailureExtProcResponse {}

#[async_trait::async_trait]
impl Handler for FailureExtProcResponse {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		_: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Err(Status::failed_precondition("injected test error"))
	}
}

#[derive(Debug, Default)]
struct RequestBodyFailureExtProc;

#[async_trait::async_trait]
impl Handler for RequestBodyFailureExtProc {
	async fn handle_request_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(request_header_response(None)).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_: &proto::HttpBody,
		_: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		Err(Status::failed_precondition("injected request body error"))
	}
}

#[derive(Debug, Default)]
struct BufferedBodyReplacementWithoutContentLengthExtProc;

#[async_trait::async_trait]
impl Handler for BufferedBodyReplacementWithoutContentLengthExtProc {
	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if body.end_of_stream {
			let _ = sender
				.send(request_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::Body(
							b"rewritten-request".to_vec().into(),
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		if body.end_of_stream {
			let _ = sender
				.send(response_body_response(Some(CommonResponse {
					body_mutation: Some(BodyMutation {
						mutation: Some(body_mutation::Mutation::Body(
							b"rewritten-response".to_vec().into(),
						)),
					}),
					..Default::default()
				})))
				.await;
		}
		Ok(())
	}
}

// Header conversion and mutation behavior.
mod header_mutations {
	use super::*;

	#[test]
	fn test_req_to_header_map() {
		let req = Request::builder()
			.header("host", "foo.com")
			.header("content-type", "application/json")
			.uri("/path?query=param")
			.method("GET")
			.body(http::Body::empty())
			.unwrap();
		let headers = super::super::req_to_header_map(&req).unwrap();
		// 2 regular headers, 4 pseudo headers (method, scheme, authority, path)
		assert_eq!(headers.headers.len(), 6);
	}

	#[tokio::test]
	async fn header_append_action_mock() {
		let mock = mock_with_header("x-test", "existing").await;
		let handler = HeaderAppendActionExtProc::new(vec![
			(
				"x-test",
				b"new-value",
				HeaderAppendAction::AppendIfExistsOrAdd,
			),
			("x-new", b"added", HeaderAppendAction::AppendIfExistsOrAdd),
		]);
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || handler.clone()),
			"{}",
		)
		.await;
		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let values: Vec<_> = res.headers().get_all("x-test").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "existing");
		assert_eq!(values[1], "new-value");
		assert_eq!(res.headers().get("x-new").unwrap(), "added");
	}
}

#[derive(Debug, Clone)]
struct HeaderAppendActionExtProc {
	headers: Vec<(String, Vec<u8>, HeaderAppendAction)>,
}

impl HeaderAppendActionExtProc {
	fn new(headers: Vec<(&str, &[u8], HeaderAppendAction)>) -> Self {
		Self {
			headers: headers
				.into_iter()
				.map(|(k, v, a)| (k.to_string(), v.to_vec(), a))
				.collect(),
		}
	}
}

#[async_trait::async_trait]
impl Handler for HeaderAppendActionExtProc {
	async fn handle_response_headers(
		&mut self,
		_: &HttpHeaders,
		sender: &Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let set_headers = self
			.headers
			.iter()
			.map(|(key, value, action)| HeaderValueOption {
				header: Some(HeaderValue {
					key: key.clone(),
					value: String::new(),
					raw_value: value.clone(),
				}),
				append: Some(true),
				append_action: (*action).into(),
			})
			.collect();

		let _ = sender
			.send(response_header_response(Some(CommonResponse {
				header_mutation: Some(HeaderMutation {
					set_headers,
					remove_headers: vec![],
				}),
				..Default::default()
			})))
			.await;
		Ok(())
	}
}

async fn mock_with_header(header_name: &str, header_value: &str) -> MockServer {
	let header_name = header_name.to_string();
	let header_value = header_value.to_string();
	let mock = wiremock::MockServer::start().await;
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| {
			wiremock::ResponseTemplate::new(200)
				.insert_header(header_name.as_str(), header_value.as_str())
		})
		.mount(&mock)
		.await;
	mock
}

async fn body_mock_with_content_length(body: &'static [u8]) -> MockServer {
	let mock = wiremock::MockServer::start().await;
	let len = body.len().to_string();
	wiremock::Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| {
			wiremock::ResponseTemplate::new(200)
				.insert_header("content-length", len.as_str())
				.set_body_raw(body.to_vec(), "application/octet-stream")
		})
		.mount(&mock)
		.await;
	mock
}

// Dynamic metadata container and extraction unit tests.
mod dynamic_metadata_extraction {
	use super::*;

	#[test]
	fn test_dynamic_metadata_extraction() {
		let mut metadata = ExtProcDynamicMetadata::default();

		metadata
			.0
			.insert("user_id".to_string(), serde_json::json!("12345"));
		metadata
			.0
			.insert("role".to_string(), serde_json::json!("admin"));
		assert_eq!(metadata.0.get("user_id").unwrap(), "12345");
		assert_eq!(metadata.0.get("role").unwrap(), "admin");
	}

	mod extract_dynamic_metadata_tests {
		use std::collections::HashMap;

		use prost_wkt_types::value::Kind;
		use prost_wkt_types::{Struct, Value};

		use super::super::super::extract_dynamic_metadata;
		use super::*;

		#[test]
		fn test_extract_creates_extension() {
			let metadata = Struct {
				fields: [(
					"user_id".to_string(),
					Value {
						kind: Some(Kind::StringValue("12345".to_string())),
					},
				)]
				.into(),
			};
			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			extract_dynamic_metadata(&mut req, &metadata).unwrap();

			let extracted = req
				.extensions()
				.get::<ExtProcDynamicMetadata>()
				.expect("metadata should be in extensions");
			assert_eq!(
				extracted.0.get("user_id"),
				Some(&serde_json::json!("12345"))
			);
		}

		#[test]
		fn test_extract_merges_with_existing() {
			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			let existing = ExtProcDynamicMetadata(
				[("existing".to_string(), serde_json::json!("value"))]
					.into_iter()
					.collect(),
			);
			req.extensions_mut().insert(existing);

			let metadata = Struct {
				fields: [(
					"new_key".to_string(),
					Value {
						kind: Some(Kind::StringValue("new_value".to_string())),
					},
				)]
				.into(),
			};
			extract_dynamic_metadata(&mut req, &metadata).unwrap();

			let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
			assert_eq!(extracted.0.len(), 2);
			assert_eq!(
				extracted.0.get("existing"),
				Some(&serde_json::json!("value"))
			);
			assert_eq!(
				extracted.0.get("new_key"),
				Some(&serde_json::json!("new_value"))
			);
		}

		#[test]
		fn test_extract_overwrites_existing_keys() {
			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			let existing = ExtProcDynamicMetadata(
				[("key".to_string(), serde_json::json!("old_value"))]
					.into_iter()
					.collect(),
			);
			req.extensions_mut().insert(existing);

			let metadata = Struct {
				fields: [(
					"key".to_string(),
					Value {
						kind: Some(Kind::StringValue("new_value".to_string())),
					},
				)]
				.into(),
			};
			extract_dynamic_metadata(&mut req, &metadata).unwrap();

			let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
			assert_eq!(extracted.0.len(), 1);
			assert_eq!(
				extracted.0.get("key"),
				Some(&serde_json::json!("new_value"))
			);
		}

		#[test]
		fn test_extract_empty_metadata_no_extension() {
			let metadata = Struct {
				fields: HashMap::new(),
			};
			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			extract_dynamic_metadata(&mut req, &metadata).unwrap();

			assert!(req.extensions().get::<ExtProcDynamicMetadata>().is_none());
		}

		#[test]
		fn test_extract_string_and_bool_values() {
			let metadata = Struct {
				fields: [
					(
						"string_val".to_string(),
						Value {
							kind: Some(Kind::StringValue("hello".to_string())),
						},
					),
					(
						"bool_true".to_string(),
						Value {
							kind: Some(Kind::BoolValue(true)),
						},
					),
					(
						"bool_false".to_string(),
						Value {
							kind: Some(Kind::BoolValue(false)),
						},
					),
				]
				.into(),
			};

			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			extract_dynamic_metadata(&mut req, &metadata).unwrap();

			let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();

			assert_eq!(extracted.0.len(), 3);
			assert_eq!(
				extracted.0.get("string_val"),
				Some(&serde_json::json!("hello"))
			);
			assert_eq!(extracted.0.get("bool_true"), Some(&serde_json::json!(true)));
			assert_eq!(
				extracted.0.get("bool_false"),
				Some(&serde_json::json!(false))
			);
		}

		#[test]
		fn test_extract_multiple_calls_accumulate() {
			let mut req = ::http::Request::builder()
				.uri("http://test.com")
				.body(Body::empty())
				.unwrap();

			let metadata1 = Struct {
				fields: [(
					"key1".to_string(),
					Value {
						kind: Some(Kind::StringValue("value1".to_string())),
					},
				)]
				.into(),
			};
			extract_dynamic_metadata(&mut req, &metadata1).unwrap();

			let metadata2 = Struct {
				fields: [(
					"key2".to_string(),
					Value {
						kind: Some(Kind::BoolValue(true)),
					},
				)]
				.into(),
			};
			extract_dynamic_metadata(&mut req, &metadata2).unwrap();

			let extracted = req.extensions().get::<ExtProcDynamicMetadata>().unwrap();
			assert_eq!(extracted.0.len(), 2);
			assert_eq!(extracted.0.get("key1"), Some(&serde_json::json!("value1")));
			assert_eq!(extracted.0.get("key2"), Some(&serde_json::json!(true)));
		}
	}
}

type RequestLog = Arc<std::sync::Mutex<Vec<proto::ProcessingRequest>>>;

fn request_log() -> RequestLog {
	Arc::new(std::sync::Mutex::new(Vec::new()))
}

#[derive(Clone)]
struct RequestRecorder {
	requests: RequestLog,
}

impl RequestRecorder {
	fn new() -> Self {
		Self {
			requests: request_log(),
		}
	}
}

type MetadataTracker = RequestRecorder;

#[async_trait::async_trait]
impl Handler for RequestRecorder {
	async fn on_request(&mut self, request: &proto::ProcessingRequest) {
		self.requests.lock().unwrap().push(request.clone());
	}

	async fn handle_request_trailers(
		&mut self,
		_trailers: &proto::HttpTrailers,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(Ok(ProcessingResponse {
				response: Some(processing_response::Response::RequestTrailers(
					proto::TrailersResponse {
						header_mutation: None,
					},
				)),
				..Default::default()
			}))
			.await;
		Ok(())
	}

	async fn handle_response_trailers(
		&mut self,
		_trailers: &proto::HttpTrailers,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(Ok(ProcessingResponse {
				response: Some(processing_response::Response::ResponseTrailers(
					proto::TrailersResponse {
						header_mutation: None,
					},
				)),
				..Default::default()
			}))
			.await;
		Ok(())
	}
}

#[derive(Clone)]
struct ModeOverrideTracker {
	requests: RequestLog,
	request_headers_override: Option<ProcessingMode>,
	response_headers_override: Option<ProcessingMode>,
	request_body_override: Option<ProcessingMode>,
}

impl ModeOverrideTracker {
	fn new() -> Self {
		Self {
			requests: request_log(),
			request_headers_override: None,
			response_headers_override: None,
			request_body_override: None,
		}
	}

	fn with_request_headers_override(mut self, mode_override: ProcessingMode) -> Self {
		self.request_headers_override = Some(mode_override);
		self
	}

	fn with_response_headers_override(mut self, mode_override: ProcessingMode) -> Self {
		self.response_headers_override = Some(mode_override);
		self
	}

	fn with_request_body_override(mut self, mode_override: ProcessingMode) -> Self {
		self.request_body_override = Some(mode_override);
		self
	}
}

fn request_headers_response_with_mode_override(
	mode_override: ProcessingMode,
) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestHeaders(
			proto::HeadersResponse { response: None },
		)),
		mode_override: Some(mode_override),
		..Default::default()
	})
}

fn response_headers_response_with_mode_override(
	mode_override: ProcessingMode,
) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::ResponseHeaders(
			proto::HeadersResponse { response: None },
		)),
		mode_override: Some(mode_override),
		..Default::default()
	})
}

fn request_body_response_with_mode_override(
	mode_override: ProcessingMode,
) -> Result<ProcessingResponse, Status> {
	Ok(ProcessingResponse {
		response: Some(processing_response::Response::RequestBody(
			proto::BodyResponse { response: None },
		)),
		mode_override: Some(mode_override),
		..Default::default()
	})
}

#[async_trait::async_trait]
impl Handler for ModeOverrideTracker {
	async fn on_request(&mut self, request: &proto::ProcessingRequest) {
		self.requests.lock().unwrap().push(request.clone());
	}

	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let response = self
			.request_headers_override
			.map(request_headers_response_with_mode_override)
			.unwrap_or_else(|| request_header_response(None));
		let _ = sender.send(response).await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let response = self
			.request_body_override
			.map(request_body_response_with_mode_override)
			.unwrap_or_else(|| request_body_response(None));
		let _ = sender.send(response).await;
		Ok(())
	}

	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let response = self
			.response_headers_override
			.map(response_headers_response_with_mode_override)
			.unwrap_or_else(|| response_header_response(None));
		let _ = sender.send(response).await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		_body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender.send(response_body_response(None)).await;
		Ok(())
	}
}

#[derive(Clone)]
struct ModeOverrideRequestBodyBufferedPartialExtProc;

#[async_trait::async_trait]
impl Handler for ModeOverrideRequestBodyBufferedPartialExtProc {
	async fn handle_request_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(request_headers_response_with_mode_override(
				ProcessingMode {
					request_body_mode: processing_mode::BodySendMode::BufferedPartial as i32,
					..Default::default()
				},
			))
			.await;
		Ok(())
	}

	async fn handle_request_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let response =
			ModeAwareBodyExtProc::body_response(&BufferedBodyMode::ReplaceAny(b"HELLO".to_vec()), body);
		let _ = sender.send(request_body_response(response)).await;
		Ok(())
	}
}

#[derive(Clone)]
struct ModeOverrideResponseBodyBufferedPartialExtProc;

#[async_trait::async_trait]
impl Handler for ModeOverrideResponseBodyBufferedPartialExtProc {
	async fn handle_response_headers(
		&mut self,
		_headers: &HttpHeaders,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let _ = sender
			.send(response_headers_response_with_mode_override(
				ProcessingMode {
					response_body_mode: processing_mode::BodySendMode::BufferedPartial as i32,
					..Default::default()
				},
			))
			.await;
		Ok(())
	}

	async fn handle_response_body(
		&mut self,
		body: &proto::HttpBody,
		sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
	) -> Result<(), Status> {
		let response =
			ModeAwareBodyExtProc::body_response(&BufferedBodyMode::ReplaceAny(b"HELLO".to_vec()), body);
		let _ = sender.send(response_body_response(response)).await;
		Ok(())
	}
}

// Mode override guardrails for unsupported STREAMED and existing full-duplex state.
mod mode_override_guardrails {
	use super::*;

	#[tokio::test]
	async fn mode_override_streamed_does_not_enable_response_body_phase() {
		let mock = body_mock(b"upstream-response").await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			response_body_mode: processing_mode::BodySendMode::Streamed as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let _ = read_body_raw(res.into_body()).await;

		let captured = requests.lock().unwrap();
		let saw_response_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseBody(_))
			)
		});
		assert!(
			!saw_response_body,
			"unsupported STREAMED mode_override must not be converted into response body processing"
		);
	}

	#[tokio::test]
	async fn mode_override_streamed_does_not_enable_request_body_phase() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			request_body_mode: processing_mode::BodySendMode::Streamed as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"abc").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let saw_request_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::RequestBody(_))
			)
		});
		assert!(
			!saw_request_body,
			"unsupported STREAMED mode_override must not be converted into request body processing"
		);
	}

	#[tokio::test]
	async fn mode_override_does_not_change_request_body_when_initial_mode_is_full_duplex() {
		let mock = simple_mock().await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			request_body_mode: processing_mode::BodySendMode::None as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "fullDuplexStreamed",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "send",
			"responseTrailerMode": "skip",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request_body(io, Method::POST, "http://lo", b"abc").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		let saw_request_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::RequestBody(_))
			)
		});
		assert!(
			saw_request_body,
			"mode_override must not disable request body processing when the initial mode is FULL_DUPLEX_STREAMED"
		);
	}

	#[tokio::test]
	async fn mode_override_does_not_change_response_body_when_initial_mode_is_full_duplex() {
		let mock = body_mock(b"upstream-response").await;
		let tracker = ModeOverrideTracker::new().with_request_headers_override(ProcessingMode {
			response_body_mode: processing_mode::BodySendMode::None as i32,
			..Default::default()
		});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "fullDuplexStreamed",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "send",
			"allowModeOverride": true,
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let _ = read_body_raw(res.into_body()).await;

		let captured = requests.lock().unwrap();
		let saw_response_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseBody(_))
			)
		});
		assert!(
			saw_response_body,
			"mode_override must not disable response body processing when the initial mode is FULL_DUPLEX_STREAMED"
		);
	}

	#[tokio::test]
	async fn mode_override_does_not_change_response_body_after_current_mode_becomes_full_duplex() {
		let mock = body_mock(b"upstream-response").await;
		let tracker = ModeOverrideTracker::new()
			.with_request_headers_override(ProcessingMode {
				response_body_mode: processing_mode::BodySendMode::FullDuplexStreamed as i32,
				response_trailer_mode: processing_mode::HeaderSendMode::Send as i32,
				..Default::default()
			})
			.with_response_headers_override(ProcessingMode {
				response_body_mode: processing_mode::BodySendMode::None as i32,
				..Default::default()
			});
		let requests = tracker.requests.clone();
		let processing_options = json!({
			"requestBodyMode": "none",
			"responseBodyMode": "none",
			"requestHeaderMode": "send",
			"responseHeaderMode": "send",
			"requestTrailerMode": "skip",
			"responseTrailerMode": "skip",
			"allowModeOverride": true
		});

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_processing_options(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(processing_options),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
		let _ = read_body_raw(res.into_body()).await;

		let captured = requests.lock().unwrap();
		let saw_response_body = captured.iter().any(|r| {
			matches!(
				r.request,
				Some(proto::processing_request::Request::ResponseBody(_))
			)
		});
		assert!(
			saw_response_body,
			"once the current mode becomes FULL_DUPLEX_STREAMED, later body mode_override values must be ignored"
		);
	}
}

// Body stream helper behavior and end-to-end trailer forwarding.
mod body_streaming_and_trailers {
	use super::*;

	#[tokio::test]
	async fn handle_body_stream_skips_trailers_when_send_trailers_is_false() {
		let mut trailers = ::http::HeaderMap::new();
		trailers.insert("x-test-trailer", "value".parse().unwrap());
		let frames = tokio_stream::iter(vec![
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::data(bytes::Bytes::from_static(b"hello"))),
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::trailers(trailers)),
		]);
		let body = Body::new(http_body_util::StreamBody::new(frames));

		let (tx, mut rx) = mpsc::channel(8);
		super::super::ExtProcInstance::handle_body_stream(
			None,
			body,
			tx,
			super::super::BodyStreamDirection::Request,
			false,
			super::super::FirstExtProcMessage::default(),
		)
		.await;

		let mut saw_trailers = false;
		while let Some(req) = rx.recv().await {
			if matches!(
				req.request,
				Some(proto::processing_request::Request::RequestTrailers(_))
			) {
				saw_trailers = true;
			}
		}
		assert!(!saw_trailers);
	}

	#[tokio::test]
	async fn handle_body_stream_sends_trailers_when_send_trailers_is_true() {
		let mut trailers = ::http::HeaderMap::new();
		trailers.insert("x-test-trailer", "value".parse().unwrap());
		let frames = tokio_stream::iter(vec![
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::data(bytes::Bytes::from_static(b"hello"))),
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::trailers(trailers.clone())),
		]);
		let body = Body::new(http_body_util::StreamBody::new(frames));

		let (tx, mut rx) = mpsc::channel(8);
		super::super::ExtProcInstance::handle_body_stream(
			None,
			body,
			tx,
			super::super::BodyStreamDirection::Request,
			true,
			super::super::FirstExtProcMessage::default(),
		)
		.await;

		let mut saw_expected_trailer = false;
		let mut saw_body_after_trailers = false;
		while let Some(req) = rx.recv().await {
			match req.request {
				Some(proto::processing_request::Request::RequestTrailers(ts)) => {
					if let Some(map) = ts.trailers
						&& map
							.headers
							.iter()
							.any(|h| h.key.eq_ignore_ascii_case("x-test-trailer"))
					{
						saw_expected_trailer = true;
					}
				},
				Some(proto::processing_request::Request::RequestBody(_)) if saw_expected_trailer => {
					saw_body_after_trailers = true;
				},
				_ => {},
			}
		}
		assert!(saw_expected_trailer);
		assert!(
			!saw_body_after_trailers,
			"trailers should be the final message for a body stream"
		);
	}

	#[tokio::test]
	async fn request_trailers_are_sent_end_to_end_when_enabled() {
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let processing_options = ext_proc::ProcessingOptions {
			request_body_mode: ext_proc::BodySendMode::FullDuplexStreamed,
			response_body_mode: ext_proc::BodySendMode::None,
			request_header_mode: ext_proc::HeaderSendMode::Send,
			response_header_mode: ext_proc::HeaderSendMode::Send,
			request_trailer_mode: ext_proc::TrailerSendMode::Send,
			response_trailer_mode: ext_proc::TrailerSendMode::Skip,
			..Default::default()
		};
		let ext_proc = ExtProcMock::new(move || tracker.clone()).spawn().await;
		let mut ext_proc_request =
			build_ext_proc_request_for_test(ext_proc.address, processing_options);

		let mut trailers = ::http::HeaderMap::new();
		trailers.insert("x-request-trailer", "request-value".parse().unwrap());
		let frames = tokio_stream::iter(vec![
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::data(bytes::Bytes::from_static(
				b"request body",
			))),
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::trailers(trailers)),
		]);
		let mut req = crate::proxy::request_builder::RequestBuilder::new(Method::POST, "http://lo")
			.body(Body::new(http_body_util::StreamBody::new(frames)))
			.build()
			.unwrap();
		let _ = ext_proc_request.mutate_request(&mut req).await.unwrap();

		let captured = requests.lock().unwrap();
		let request_body_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::RequestBody(_))
				)
			})
			.expect("request body should be sent before request trailers");
		let request_trailers_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::RequestTrailers(_))
				)
			})
			.expect("request trailers should be sent when requestTrailerMode=send");
		assert!(
			request_body_pos < request_trailers_pos,
			"request trailers should arrive after request body chunks"
		);
	}

	#[tokio::test]
	async fn response_trailers_are_sent_end_to_end_when_enabled() {
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();
		let ext_proc = ExtProcMock::new(move || tracker.clone()).spawn().await;
		let processing_options = ext_proc::ProcessingOptions {
			request_body_mode: ext_proc::BodySendMode::None,
			response_body_mode: ext_proc::BodySendMode::FullDuplexStreamed,
			request_header_mode: ext_proc::HeaderSendMode::Send,
			response_header_mode: ext_proc::HeaderSendMode::Send,
			request_trailer_mode: ext_proc::TrailerSendMode::Skip,
			response_trailer_mode: ext_proc::TrailerSendMode::Send,
			..Default::default()
		};
		let mut ext_proc_request =
			build_ext_proc_request_for_test(ext_proc.address, processing_options);

		let mut trailers = ::http::HeaderMap::new();
		trailers.insert("x-response-trailer", "response-value".parse().unwrap());
		let frames = tokio_stream::iter(vec![
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::data(bytes::Bytes::from_static(
				b"upstream-response",
			))),
			Ok::<Frame<bytes::Bytes>, Infallible>(Frame::trailers(trailers)),
		]);
		let mut resp = http::Response::new(Body::new(http_body_util::StreamBody::new(frames)));
		let _ = ext_proc_request
			.mutate_response(&mut resp, None)
			.await
			.unwrap();
		let body = read_body_raw(resp.into_body()).await;
		assert_eq!(body.as_ref(), b"upstream-response");

		let captured = requests.lock().unwrap();
		let response_body_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseBody(_))
				)
			})
			.expect("response body should be sent before response trailers");
		let response_trailers_pos = captured
			.iter()
			.position(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseTrailers(_))
				)
			})
			.expect("response trailers should be sent when responseTrailerMode=send");
		assert!(
			response_body_pos < response_trailers_pos,
			"response trailers should arrive after response body chunks"
		);
	}
}

// CEL metadata_context and ext_proc request/response attribute evaluation.
mod metadata_context_and_attributes {
	use super::*;

	#[tokio::test]
	async fn test_attributes_empty_without_config() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
		)
		.await;
		let res = send_request_body(io, Method::POST, "http://lo", b"request body").await;
		assert_eq!(res.status(), 200);

		let captured = requests.lock().unwrap();
		assert!(captured.len() >= 2);

		for (i, req) in captured.iter().enumerate() {
			assert!(
				req.attributes.is_empty(),
				"Message {} should have empty attributes when no config",
				i
			);
		}
	}

	struct DynamicMetadataResponder;

	#[async_trait::async_trait]
	impl Handler for DynamicMetadataResponder {
		async fn handle_request_headers(
			&mut self,
			_headers: &HttpHeaders,
			sender: &mpsc::Sender<Result<ProcessingResponse, Status>>,
		) -> Result<(), Status> {
			use prost_wkt_types::value::Kind;
			use prost_wkt_types::{Struct, Value};

			use crate::test_helpers::extprocmock::request_header_response_with_dynamic_metadata;

			let metadata = Struct {
				fields: [
					(
						"auth_user".to_string(),
						Value {
							kind: Some(Kind::StringValue("test-user".to_string())),
						},
					),
					(
						"is_admin".to_string(),
						Value {
							kind: Some(Kind::BoolValue(true)),
						},
					),
				]
				.into(),
			};
			let _ = sender
				.send(request_header_response_with_dynamic_metadata(
					None, metadata,
				))
				.await;
			Ok(())
		}
	}

	#[tokio::test]
	async fn test_dynamic_metadata_response() {
		let mock = simple_mock().await;
		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(|| DynamicMetadataResponder),
			"{}",
		)
		.await;
		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);
	}

	#[tokio::test]
	async fn test_cel_metadata_context_evaluation() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();

		let meta = HashMap::from([(
			"envoy.filters.http.ext_proc".to_string(),
			[
				(
					"path".to_string(),
					Arc::new(Expression::new_strict("request.path").unwrap()),
				),
				(
					"static".to_string(),
					Arc::new(Expression::new_strict("'value'").unwrap()),
				),
			]
			.into(),
		)]);

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			Some(meta),
			None,
			None,
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo/test-path").await;
		assert_eq!(res.status(), 200);

		let reqs = requests.lock().unwrap();
		assert!(!reqs.is_empty());

		let req = &reqs[0];
		let meta_ctx = req
			.metadata_context
			.as_ref()
			.expect("should have metadata_context");
		let filter_meta = meta_ctx
			.filter_metadata
			.get("envoy.filters.http.ext_proc")
			.expect("should have namespace");

		let fields = &filter_meta.fields;
		match &fields.get("path").unwrap().kind {
			Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "/test-path"),
			invalid => panic!("exepected a string 'path' got {:?}", invalid),
		}
		match &fields.get("static").unwrap().kind {
			Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "value"),
			invalid => panic!("exepected a string 'static' field got {:?}", invalid),
		}
	}

	#[tokio::test]
	async fn test_cel_req_attributes() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			None,
			Some(
				[(
					"method".to_string(),
					Arc::new(Expression::new_strict("request.method").unwrap()),
				)]
				.into(),
			),
			None,
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let req = requests.lock().unwrap();
		let headers = req
			.iter()
			.find(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::RequestHeaders(_))
				)
			})
			.unwrap();

		let ns_attrs = &headers
			.attributes
			.get("envoy.filters.http.ext_proc")
			.expect("envoy ext_proc namespace");

		match &ns_attrs.fields.get("method").unwrap().kind {
			Some(prost_wkt_types::value::Kind::StringValue(s)) => assert_eq!(s, "GET"),
			invalid => panic!("exepected a string got {:?}", invalid),
		}
	}

	#[tokio::test]
	async fn test_cel_resp_attributes() {
		let mock = simple_mock().await;
		let tracker = MetadataTracker::new();
		let requests = tracker.requests.clone();

		let (_mock, _ext_proc, _bind, io) = setup_ext_proc_mock_with_meta(
			mock,
			ext_proc::FailureMode::FailClosed,
			ExtProcMock::new(move || tracker.clone()),
			"{}",
			None,
			None,
			Some(
				[(
					"status".to_string(),
					Arc::new(Expression::new_strict("response.code").unwrap()),
				)]
				.into(),
			),
		)
		.await;

		let res = send_request(io, Method::GET, "http://lo").await;
		assert_eq!(res.status(), 200);

		let resp = requests.lock().unwrap();
		let headers = resp
			.iter()
			.find(|r| {
				matches!(
					r.request,
					Some(proto::processing_request::Request::ResponseHeaders(_))
				)
			})
			.unwrap();

		let ns_attrs = &headers
			.attributes
			.get("envoy.filters.http.ext_proc")
			.expect("envoy ext_proc namespace");

		match &ns_attrs.fields.get("status").unwrap().kind {
			Some(prost_wkt_types::value::Kind::NumberValue(n)) => assert_eq!(*n, 200.0),
			invalid => panic!("exepected a number got {:?}", invalid),
		}
	}
}
