use std::convert::Infallible;
use std::sync::Mutex;

use anyhow::anyhow;
use bytes::Bytes;
use http_body::{Body, Frame};
use http_body_util::BodyExt;
use prost_wkt_types::Struct;
use proto::processing_request::Request;
use proto::processing_response::Response;
use protos::envoy::service::ext_proc::v3::ProtocolConfiguration;
use serde_json::Value as JsonValue;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;

use crate::cel::{Executor, Expression, RequestSnapshot};
use crate::client::ResolvedDestination;
use crate::http::ext_proc::proto::{
	HttpBody, HttpHeaders, HttpTrailers, Metadata, ProcessingRequest, ProcessingResponse,
	processing_response,
};
use crate::http::{HeaderName, PolicyResponse, envoy_proto_common};
use crate::proxy::ProxyError;
use crate::proxy::dtrace::{self, pol_result};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};
use crate::{http, *};

/// The namespace key used for ext_proc attributes in ProcessingRequest.attributes
const EXTPROC_ATTRIBUTES_NAMESPACE: &str = "envoy.filters.http.ext_proc";

#[cfg(test)]
#[path = "ext_proc_tests.rs"]
mod tests;

const TRACE_POLICY_KIND: &str = "ext_proc";

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("failed to send request")]
	RequestSend,
	#[error("no more response messages")]
	NoMoreResponses,
	#[error("no more responses")]
	ResponseDropped,
	#[error("failed to buffer body: {0}")]
	BodyBuffer(String),
	#[error("invalid body mutation: {0}")]
	BodyMutation(String),
	#[error("failed to convert metadata value: {0}")]
	MetadataConversion(String),
	#[error(transparent)]
	InvalidHeaderName(#[from] http::header::InvalidHeaderName),
	#[error(transparent)]
	InvalidHeaderValue(#[from] http::header::InvalidHeaderValue),
}

#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct ExtProcDynamicMetadata(serde_json::Map<String, JsonValue>);

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	pub use protos::envoy::service::common::v3::{
		HeaderValue, HeaderValueOption, HttpStatus, Metadata, StatusCode, header_value_option,
	};
	pub use protos::envoy::service::ext_proc::v3::*;
}

mod buffering;
mod channel;
mod headers;
mod mutation;
mod processing;

use buffering::{
	BufferedBodyPhase, PendingBufferedBody, attach_request_body_channel,
	debug_assert_preserved_request_body, start_buffered_request_body, start_buffered_response_body,
};
pub use channel::GrpcReferenceChannel;
use headers::{req_to_header_map, resp_to_header_map, to_header_map};
#[cfg(test)]
use mutation::extract_dynamic_metadata;
use mutation::{
	handle_response_for_request_mutation, handle_response_for_response_mutation,
	request_body_response_has_no_mutation, request_response_has_streamed_body_mutation,
	response_body_response_has_no_mutation, response_response_has_streamed_body_mutation,
	to_immediate_response,
};
use processing::{
	BodyPath, BodyStreamDirection, FirstExtProcMessage, HeaderPhase, ModeStateMachine,
	RequestFlowFsm, RequestLoopStep, RequestPhase, ResponseFlowFsm, ResponseLoopMessage,
	ResponsePhase,
};
pub use processing::{
	BodySendMode, FailureMode, HeaderSendMode, ProcessingOptions, TrailerSendMode,
};

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
/// Controls how an endpoint-picker-selected destination is used.
pub enum InferenceRoutingDestinationMode {
	/// Require the selected destination to match agentgateway's local service endpoints.
	#[default]
	Validated,
	/// Trust the selected destination directly without local endpoint validation.
	Passthrough,
}

#[apply(schema_ser_schema!)]
pub struct InferenceRouting {
	/// Endpoint picker backend that selects the destination endpoint.
	#[serde(rename = "endpointPicker")]
	pub target: Arc<SimpleBackendReference>,
	/// How to use the destination returned by the endpoint picker.
	#[serde(
		default,
		rename = "destinationMode",
		skip_serializing_if = "crate::serdes::is_default"
	)]
	pub destination_mode: InferenceRoutingDestinationMode,
	/// Behavior when endpoint picking fails.
	#[serde(default, skip_serializing_if = "crate::serdes::is_default")]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub failure_mode: FailureMode,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InferenceRoutingConfig {
	/// Endpoint picker backend that selects the destination endpoint.
	endpoint_picker: Arc<SimpleBackendReference>,
	/// How to use the destination returned by the endpoint picker.
	#[serde(default)]
	destination_mode: InferenceRoutingDestinationMode,
}

impl<'de> serde::Deserialize<'de> for InferenceRouting {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let InferenceRoutingConfig {
			endpoint_picker,
			destination_mode,
		} = InferenceRoutingConfig::deserialize(deserializer)?;
		Ok(Self {
			target: endpoint_picker,
			destination_mode,
			// TODO: expose fail-open configuration for standalone EPP once the fallback behavior is
			// explicitly supported and documented end-to-end.
			failure_mode: FailureMode::FailClosed,
		})
	}
}

#[derive(Debug, Default)]
pub struct InferencePoolRouter {
	ext_proc: Option<ExtProcInstance>,
	destination_mode: InferenceRoutingDestinationMode,
}

#[derive(Debug, Default)]
pub struct InferenceRequestResult {
	pub destination: Option<SocketAddr>,
	pub destination_mode: InferenceRoutingDestinationMode,
	pub policy_response: PolicyResponse,
	pub failed_open: bool,
}

impl InferenceRouting {
	pub fn build(&self, client: PolicyClient) -> InferencePoolRouter {
		InferencePoolRouter {
			destination_mode: self.destination_mode,
			ext_proc: Some(ExtProcInstance::new(
				client,
				Vec::new(),
				self.target.clone(),
				self.failure_mode,
				None,
				None,
				None,
				ProcessingOptions {
					request_body_mode: BodySendMode::FullDuplexStreamed,
					response_body_mode: BodySendMode::FullDuplexStreamed,
					request_trailer_mode: TrailerSendMode::Send,
					response_trailer_mode: TrailerSendMode::Send,
					..Default::default()
				},
			)),
		}
	}
}

impl InferencePoolRouter {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<InferenceRequestResult, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(Default::default());
		};
		let r = std::mem::take(req);
		let (new_req, pr) = ext_proc.mutate_request(r).await?;
		let failed_open = ext_proc.skipped;
		*req = new_req;
		let dest = req
			.headers()
			.get(HeaderName::from_static("x-gateway-destination-endpoint"))
			.and_then(|v| v.to_str().ok())
			.map(|v| v.parse::<SocketAddr>())
			.transpose()
			.map_err(|e| ProxyError::Processing(anyhow!("EPP returned invalid address: {e}")))?;
		Ok(InferenceRequestResult {
			destination: dest,
			destination_mode: self.destination_mode,
			policy_response: pr.unwrap_or_default(),
			failed_open,
		})
	}

	pub async fn mutate_response(
		&mut self,
		resp: &mut http::Response,
	) -> Result<PolicyResponse, ProxyError> {
		let rd = resp.extensions().get::<ResolvedDestination>().map(|d| d.0);
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(Default::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r, None, rd).await?;
		*resp = new_resp;
		Ok(pr.unwrap_or_default())
	}
}

#[apply(schema!)]
pub struct ExtProc {
	/// Backend that receives external processing calls.
	#[serde(flatten)]
	pub target: Arc<SimpleBackendReference>,
	/// Backend policies used when connecting to the external processing service.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<BackendTrafficPolicy>,
	/// Behavior when the external processing service is unavailable or returns an error.
	#[serde(default)]
	pub failure_mode: FailureMode,

	/// Additional metadata to send to the external processing service.
	/// Maps to the `metadata_context.filter_metadata` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,

	/// Maps to the request `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	/// Maps to the response `attributes` field in ProcessingRequest, and allows dynamic CEL expressions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	/// Controls which request and response parts are sent to the external processing service.
	#[serde(default)]
	pub processing_options: ProcessingOptions,
}

impl ExtProc {
	pub fn build(&self, client: PolicyClient) -> ExtProcRequest {
		ExtProcRequest {
			ext_proc: Some(ExtProcInstance::new(
				client,
				self.policies.clone(),
				self.target.clone(),
				self.failure_mode,
				self.metadata_context.clone(),
				self.request_attributes.clone(),
				self.response_attributes.clone(),
				self.processing_options,
			)),
		}
	}

	pub fn expressions(&self) -> Box<dyn Iterator<Item = &Expression> + '_> {
		Box::new(
			self
				.metadata_context
				.iter()
				.flat_map(|m| {
					m.values()
						.flat_map(|inner| inner.values().map(AsRef::as_ref))
				})
				.chain(
					self
						.request_attributes
						.iter()
						.chain(self.response_attributes.iter())
						.flat_map(|m| m.values().map(AsRef::as_ref)),
				),
		)
	}
}

impl crate::store::HasExpressions for ExtProc {
	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		ExtProc::expressions(self)
	}
}

#[derive(Debug)]
pub struct ExtProcRequest {
	ext_proc: Option<ExtProcInstance>,
}

impl ExtProcRequest {
	pub async fn mutate_request(
		&mut self,
		req: &mut http::Request,
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(PolicyResponse::default());
		};
		let r = std::mem::take(req);
		let (new_req, pr) = ext_proc.mutate_request(r).await?;
		*req = new_req;
		let pr = pr.unwrap_or_default();
		pol_result!(
			dtrace::Info,
			Apply,
			"ext_proc request ({})",
			dtrace::policy_response_details(&pr)
		);
		Ok(pr)
	}

	pub async fn mutate_response(
		&mut self,
		resp: &mut http::Response,
		request: Option<&RequestSnapshot>,
	) -> Result<PolicyResponse, ProxyError> {
		let Some(ext_proc) = &mut self.ext_proc else {
			return Ok(PolicyResponse::default());
		};
		let r = std::mem::take(resp);
		let (new_resp, pr) = ext_proc.mutate_response(r, request, None).await?;
		*resp = new_resp;
		let pr = pr.unwrap_or_default();
		pol_result!(
			dtrace::Info,
			Apply,
			"ext_proc response ({})",
			dtrace::policy_response_details(&pr)
		);
		Ok(pr)
	}

	pub fn take_body_immediate_response(&self) -> Option<http::Response> {
		self
			.ext_proc
			.as_ref()?
			.request_body_immediate_response
			.lock()
			.unwrap()
			.take()
	}
}

// Very experimental support for ext_proc
#[derive(Debug)]
struct ExtProcInstance {
	failure_mode: FailureMode,
	skipped: bool,
	request_body_immediate_response: Arc<Mutex<Option<http::Response>>>,
	protocol_config_sent: bool,
	mode_state: ModeStateMachine,
	tx_req: Sender<ProcessingRequest>,
	rx_resp_for_request: Option<Receiver<ProcessingResponse>>,
	rx_resp_for_response: Option<Receiver<ProcessingResponse>>,
	metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,
	req_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
	resp_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
}

impl ExtProcInstance {
	#[allow(clippy::too_many_arguments)]
	fn new(
		client: PolicyClient,
		policies: Vec<BackendTrafficPolicy>,
		target: Arc<SimpleBackendReference>,
		failure_mode: FailureMode,
		metadata_context: Option<HashMap<String, HashMap<String, Arc<cel::Expression>>>>,
		req_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
		resp_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
		processing_options: ProcessingOptions,
	) -> ExtProcInstance {
		trace!("connecting to {:?}", target);
		let chan = GrpcReferenceChannel {
			target,
			client: client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::ExtProc),
			policies: Arc::new(policies),
		};
		let mut c = proto::external_processor_client::ExternalProcessorClient::new(chan);
		let (tx_req, rx_req) = tokio::sync::mpsc::channel(10);
		let (tx_resp, mut rx_resp) = tokio::sync::mpsc::channel(10);
		let req_stream = tokio_stream::wrappers::ReceiverStream::new(rx_req);
		tokio::task::spawn(async move {
			// Spawn a task to handle processing requests.
			// Incoming requests get send to tx_req and will be piped through here.
			let responses = match c.process(req_stream).await {
				Ok(r) => r,
				Err(e) => {
					warn!(?failure_mode, "failed to initialize extproc client: {e:?}");
					return;
				},
			};
			trace!("initial stream established");
			let mut responses = responses.into_inner();
			while let Ok(Some(item)) = responses.message().await {
				trace!("received response item {item:?}");
				let _ = tx_resp.send(item).await;
			}
		});
		let (tx_resp_for_request, rx_resp_for_request) = tokio::sync::mpsc::channel(1);
		let (tx_resp_for_response, rx_resp_for_response) = tokio::sync::mpsc::channel(1);
		tokio::task::spawn(async move {
			while let Some(item) = rx_resp.recv().await {
				match &item.response {
					Some(processing_response::Response::ResponseBody(_))
					| Some(processing_response::Response::ResponseHeaders(_))
					| Some(processing_response::Response::ResponseTrailers(_)) => {
						let _ = tx_resp_for_response.send(item).await;
					},
					Some(processing_response::Response::RequestBody(_))
					| Some(processing_response::Response::RequestHeaders(_))
					| Some(processing_response::Response::RequestTrailers(_)) => {
						let _ = tx_resp_for_request.send(item).await;
					},
					Some(processing_response::Response::ImmediateResponse(_)) => {
						// In this case we aren't sure which is going to handle things...
						// Send to both
						let _ = tx_resp_for_request.send(item.clone()).await;
						let _ = tx_resp_for_response.send(item).await;
					},
					None => {},
				}
			}
		});
		Self {
			skipped: Default::default(),
			request_body_immediate_response: Arc::new(Mutex::new(None)),
			failure_mode,
			protocol_config_sent: false,
			mode_state: processing_options.into(),
			tx_req,
			rx_resp_for_request: Some(rx_resp_for_request),
			rx_resp_for_response: Some(rx_resp_for_response),
			metadata_context,
			req_attributes,
			resp_attributes,
		}
	}

	async fn send_request(&mut self, req: ProcessingRequest) -> Result<(), Error> {
		self.tx_req.send(req).await.map_err(|_| Error::RequestSend)
	}

	fn protocol_config_for_headers(
		&self,
		protocol_config: ProtocolConfiguration,
	) -> (Option<ProtocolConfiguration>, bool) {
		let sends_protocol_config = !self.protocol_config_sent;
		(
			sends_protocol_config.then_some(protocol_config),
			sends_protocol_config,
		)
	}

	fn mark_protocol_config_sent_if(&mut self, sent: bool) {
		if sent {
			self.protocol_config_sent = true;
		}
	}

	async fn send_pending_buffered_body(
		&mut self,
		buffered_body: &mut Option<BufferedBodyPhase>,
		metadata_context: &Option<Arc<Metadata>>,
		first_message: &mut FirstExtProcMessage,
		body_direction: BodyStreamDirection,
		send_trailers: bool,
	) -> Result<bool, Error> {
		let Some(pending) = BufferedBodyPhase::take_pending_send(buffered_body).await? else {
			return Ok(false);
		};
		match pending {
			PendingBufferedBody::Body {
				body,
				handle,
				error_message,
			} => {
				let (first_message, sends_protocol_config) =
					FirstExtProcMessage::take_for_send(first_message);
				Self::send_body_stream(
					metadata_context.clone(),
					body,
					self.tx_req.clone(),
					body_direction,
					send_trailers,
					first_message,
					error_message,
				)
				.await?;
				self.mark_protocol_config_sent_if(sends_protocol_config);
				BufferedBodyPhase::replay_from_handle(buffered_body, handle)?;
			},
			PendingBufferedBody::Partial {
				body,
				end_stream,
				trailers,
			} => {
				let (first_message, sends_protocol_config) =
					FirstExtProcMessage::take_for_send(first_message);
				Self::send_partial_body(
					metadata_context.clone(),
					self.tx_req.clone(),
					body_direction,
					body,
					end_stream,
					trailers,
					send_trailers,
					first_message,
				)
				.await?;
				self.mark_protocol_config_sent_if(sends_protocol_config);
			},
		}
		Ok(true)
	}

	#[allow(clippy::too_many_arguments)]
	async fn send_partial_body(
		metadata_context: Option<Arc<Metadata>>,
		tx: Sender<ProcessingRequest>,
		body_direction: BodyStreamDirection,
		body: bytes::Bytes,
		end_stream: bool,
		trailers: Option<::http::HeaderMap>,
		send_trailers: bool,
		first_message: FirstExtProcMessage,
	) -> Result<(), Error> {
		let mut first_message = first_message;
		tx.send(ProcessingRequest {
			request: Some(body_direction.body_message(HttpBody {
				body,
				end_of_stream: end_stream,
			})),
			metadata_context: metadata_context.as_deref().cloned(),
			attributes: first_message.take_attributes_or_default(),
			protocol_config: first_message.take_protocol_config(),
			observability_mode: false,
		})
		.await
		.map_err(|_| Error::RequestSend)?;

		let Some(trailers) = trailers else {
			return Ok(());
		};
		if send_trailers {
			tx.send(ProcessingRequest {
				request: Some(body_direction.trailers_message(HttpTrailers {
					trailers: to_header_map(&trailers),
				})),
				metadata_context: metadata_context.as_deref().cloned(),
				attributes: first_message.take_attributes_or_default(),
				protocol_config: first_message.take_protocol_config(),
				observability_mode: false,
			})
			.await
			.map_err(|_| Error::RequestSend)?;
			return Ok(());
		}
		tx.send(ProcessingRequest {
			request: Some(body_direction.body_message(HttpBody {
				body: Default::default(),
				end_of_stream: true,
			})),
			metadata_context: metadata_context.and_then(Arc::into_inner),
			attributes: first_message.take_attributes_or_default(),
			protocol_config: first_message.take_protocol_config(),
			observability_mode: false,
		})
		.await
		.map_err(|_| Error::RequestSend)?;
		Ok(())
	}

	#[allow(clippy::too_many_arguments)]
	fn spawn_body_stream(
		&self,
		metadata_context: &Option<Arc<Metadata>>,
		pending_body: &mut Option<http::Body>,
		tx: Sender<ProcessingRequest>,
		body_direction: BodyStreamDirection,
		send_trailers: bool,
		first_message: FirstExtProcMessage,
		missing_body_message: &'static str,
	) {
		tokio::task::spawn(Self::handle_body_stream(
			metadata_context.clone(),
			pending_body.take().expect(missing_body_message),
			tx,
			body_direction,
			send_trailers,
			first_message,
		));
	}

	async fn recv_response_loop_message(
		rx: &mut Receiver<ProcessingResponse>,
	) -> Result<ResponseLoopMessage, Error> {
		let Some(presp) = rx.recv().await else {
			trace!("done receiving response");
			return Err(Error::NoMoreResponses);
		};
		if let Some(dr) = to_immediate_response(&presp) {
			trace!("got immediate response in request handler");
			return Ok(ResponseLoopMessage::Immediate(dr));
		}
		Ok(ResponseLoopMessage::Processing(presp))
	}

	async fn process_response_loop_message(
		&mut self,
		presp: ProcessingResponse,
		resp: Option<&mut http::Response>,
		tx_chunk: &mut Sender<Result<Frame<Bytes>, Infallible>>,
		response_fsm: &mut ResponseFlowFsm,
		send_response_headers: bool,
		ignore_mode_override: bool,
	) -> Result<(bool, bool), Error> {
		if matches!(presp.response, Some(Response::ResponseHeaders(_))) {
			self
				.mode_state
				.mark_headers_processed(HeaderPhase::Response);
			if ignore_mode_override && presp.mode_override.is_some() {
				warn!("received mode_override after full-duplex response body streaming started; ignoring");
			} else {
				self.maybe_apply_mode_override(HeaderPhase::Response, &presp);
			}
			response_fsm.reconcile_potential_mode_override(self.mode_state.response_body_mode);
		}
		let (headers_done, eos) = handle_response_for_response_mutation(
			response_fsm.send_body,
			send_response_headers,
			response_fsm
				.body_path
				.validates_content_length(send_response_headers),
			resp,
			tx_chunk,
			presp,
		)
		.await?;
		Ok((response_fsm.advance_after_response(headers_done), eos))
	}

	async fn process_request_loop_message(
		&mut self,
		presp: ProcessingResponse,
		req: Option<&mut http::Request>,
		tx_chunk: &mut Sender<Result<Frame<Bytes>, Infallible>>,
		request_fsm: &mut RequestFlowFsm,
		send_request_headers: bool,
		ignore_mode_override: bool,
	) -> Result<RequestLoopStep, Error> {
		let body_no_mutation = request_body_response_has_no_mutation(&presp);
		let streamed_body_mutation = request_response_has_streamed_body_mutation(&presp);
		if matches!(presp.response, Some(Response::RequestHeaders(_))) {
			self.mode_state.mark_headers_processed(HeaderPhase::Request);
			if ignore_mode_override && presp.mode_override.is_some() {
				warn!("received mode_override after full-duplex request body streaming started; ignoring");
			} else {
				self.maybe_apply_mode_override(HeaderPhase::Request, &presp);
			}
			request_fsm.reconcile_potential_mode_override(self.mode_state.request_body_mode);
		}
		let (headers_done, eos) = handle_response_for_request_mutation(
			request_fsm.expect_body_response,
			send_request_headers,
			request_fsm
				.body_path
				.validates_content_length(send_request_headers),
			req,
			tx_chunk,
			presp,
		)
		.await?;
		Ok(RequestLoopStep {
			transitioned: request_fsm.advance_after_response(headers_done),
			eos,
			body_no_mutation,
			streamed_body_mutation,
		})
	}

	async fn forward_response_stream_continuation(
		mut rx: Receiver<ProcessingResponse>,
		mut tx_chunk: Sender<Result<Frame<Bytes>, Infallible>>,
		send_response_body: bool,
		send_response_headers: bool,
	) {
		loop {
			let Some(presp) = rx.recv().await else {
				trace!("done receiving response");
				return;
			};
			let response = handle_response_for_response_mutation(
				send_response_body,
				send_response_headers,
				false,
				None,
				&mut tx_chunk,
				presp,
			)
			.await;
			let (_, eos) = match response {
				Ok(response) => response,
				Err(error) => {
					warn!("response stream mutation failed: {error}");
					return;
				},
			};
			if eos || !send_response_body {
				trace!("response EOS!");
				drop(tx_chunk);
				return;
			}
		}
	}

	fn protocol_config(&self) -> ProtocolConfiguration {
		ProtocolConfiguration {
			request_body_mode: self.mode_state.request_body_mode.into(),
			response_body_mode: self.mode_state.response_body_mode.into(),
			..Default::default()
		}
	}

	fn maybe_apply_mode_override(&mut self, phase: HeaderPhase, presp: &ProcessingResponse) {
		let Some(mode_override) = presp.mode_override.as_ref() else {
			return;
		};
		// This updates mode_state (protocol configuration). RequestFlowFsm/
		// ResponseFlowFsm are synced from mode_state when we consume header-phase
		// responses in the mutate_* loops.
		let valid_phase_response = matches!(
			(phase, &presp.response),
			(
				HeaderPhase::Request,
				Some(processing_response::Response::RequestHeaders(_))
			) | (
				HeaderPhase::Response,
				Some(processing_response::Response::ResponseHeaders(_))
			)
		);
		if !valid_phase_response {
			warn!(phase = ?phase, "received mode_override outside matching headers response phase; ignoring");
			return;
		}
		if !self.mode_state.allow_mode_override {
			warn!("received mode_override but allow_mode_override is disabled; ignoring");
			return;
		}
		self
			.mode_state
			.apply_envoy_mode_override(phase, mode_override);
	}

	pub async fn mutate_request(
		&mut self,
		mut req: http::Request,
	) -> Result<(http::Request, Option<PolicyResponse>), Error> {
		let headers = req_to_header_map(&req);

		let exec = cel::Executor::new_request(&req);
		// request_attributes should only be sent on first ProcessingRequest
		// this will need to be modified if we configure which Requests to send
		// Wrap metadata_context in Arc for cheap cloning across body chunks
		let metadata_context = self.metadata_context.as_ref().map(|meta| {
			Arc::new(Metadata {
				filter_metadata: meta
					.iter()
					.filter_map(|(n, e)| {
						eval_to_struct(&exec, e).map(|v| (n.clone(), v)).ok() // TODO(mk): where best to log convertion issues
					})
					.collect(),
			})
		});
		let attributes = self
			.req_attributes
			.as_ref()
			.and_then(|attrs| {
				eval_to_struct(&exec, attrs)
					.map(|v| HashMap::from([(EXTPROC_ATTRIBUTES_NAMESPACE.to_string(), v)]))
					.ok()
			})
			.unwrap_or_default();

		let failure_mode = self.failure_mode;
		let end_of_stream = req.body().is_end_stream();
		let had_body = !end_of_stream;
		let send_request_headers = self.mode_state.request_header_mode == HeaderSendMode::Send;
		let req_body_mode = self.mode_state.request_body_mode;
		// request_fsm is method-local execution state (what phase mutate_request is
		// waiting on). mode_state is protocol state that can be updated by
		// mode_override responses.
		let mut request_fsm = RequestFlowFsm::new(send_request_headers, had_body, req_body_mode);
		let protocol_config = self.protocol_config();
		// If request headers are skipped, the first body-phase ProcessingRequest must carry
		// attributes/protocol_config, and subsequent ProcessingRequests must not.
		let mut first_message_when_headers_skipped = FirstExtProcMessage::for_body_phase(
			send_request_headers,
			request_fsm.expect_body_response,
			attributes.clone(),
			protocol_config,
			self.protocol_config_sent,
		);

		// Send request headers unless processing options explicitly skip this phase.
		if send_request_headers {
			let (header_protocol_config, sends_protocol_config) =
				self.protocol_config_for_headers(protocol_config);
			if let Err(e) = self
				.send_request(ProcessingRequest {
					request: Some(Request::RequestHeaders(HttpHeaders {
						headers,
						end_of_stream,
					})),
					metadata_context: metadata_context.as_deref().cloned(),
					attributes: attributes.clone(),
					protocol_config: header_protocol_config,
					observability_mode: false,
				})
				.await
			{
				if failure_mode == FailureMode::FailOpen {
					trace!("fail open triggered");
					self.skipped = true;
					debug_assert_preserved_request_body(
						&req,
						had_body,
						"fail_open_after_request_header_send_failure_preserves_original_body",
					);
					return Ok((req, None));
				}
				return Err(e);
			}
			self.mark_protocol_config_sent_if(sends_protocol_config);
		}

		if request_fsm.phase == RequestPhase::Complete {
			// Nothing to send for request-side processing. Keep the original request intact; this
			// includes initial requestBodyMode=None when request headers are skipped.
			debug_assert_preserved_request_body(
				&req,
				had_body,
				"complete_request_phase_preserves_original_body",
			);
			return Ok((req, None));
		}

		let tx = self.tx_req.clone();
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);
		let mut rx_chunk = Some(rx_chunk);
		let mut pending_full_duplex_body = None;
		let mut request_body_started_to_ext_proc = false;

		// FULL_DUPLEX_STREAMED sends body chunks as they arrive. The ext_proc server may buffer
		// the headers and complete body before sending any response, so waiting for the headers
		// response here would deadlock that valid server behavior.
		let request_body_streamed_before_header_response =
			req_body_mode == BodySendMode::FullDuplexStreamed && had_body;
		if request_body_streamed_before_header_response {
			let (req_with_channel, body) = attach_request_body_channel(req, &mut rx_chunk);
			req = req_with_channel;
			pending_full_duplex_body = Some(body);
			let (first_message, sends_protocol_config) =
				FirstExtProcMessage::take_for_send(&mut first_message_when_headers_skipped);
			let send_request_trailers = self.mode_state.request_trailer_mode == TrailerSendMode::Send;
			self.spawn_body_stream(
				&metadata_context,
				&mut pending_full_duplex_body,
				tx.clone(),
				BodyStreamDirection::Request,
				send_request_trailers,
				first_message,
				"request body should be available before streaming starts",
			);
			self.mark_protocol_config_sent_if(sends_protocol_config);
			request_body_started_to_ext_proc = true;
		}

		let mut rx = self
			.rx_resp_for_request
			.take()
			.expect("mutate_request called twice");
		// For buffered modes: send the held body after headers_done. Buffered drains to EOF (fails out if we exceed the limit);
		// BufferedPartial sends one prefix and keeps the remainder for local pass-through.
		let mut pending_buffered_body = None;
		if request_fsm.phase != RequestPhase::AwaitingHeaders
			&& matches!(
				request_fsm.body_path,
				BodyPath::Buffered | BodyPath::BufferedPartial
			) && pending_buffered_body.is_none()
			&& rx_chunk.is_some()
		{
			let (req_with_channel, buffered_body) = start_buffered_request_body(
				req,
				self.mode_state.request_body_mode,
				had_body,
				&mut rx_chunk,
			);
			req = req_with_channel;
			pending_buffered_body = buffered_body;
		}
		if request_fsm.phase != RequestPhase::AwaitingHeaders {
			let send_request_trailers = self.mode_state.request_trailer_mode == TrailerSendMode::Send;
			let sending_buffered_body = pending_buffered_body.is_some();
			match self
				.send_pending_buffered_body(
					&mut pending_buffered_body,
					&metadata_context,
					&mut first_message_when_headers_skipped,
					BodyStreamDirection::Request,
					send_request_trailers,
				)
				.await
			{
				Ok(true) => request_body_started_to_ext_proc = true,
				Ok(false) => {},
				Err(e) => {
					if failure_mode == FailureMode::FailOpen
						&& !request_body_started_to_ext_proc
						&& !sending_buffered_body
					{
						trace!("fail open triggered");
						self.skipped = true;
						debug_assert_preserved_request_body(
							&req,
							had_body && rx_chunk.is_some(),
							"fail_open_before_request_body_phase_preserves_original_body",
						);
						return Ok((req, None));
					}
					return Err(e);
				},
			}
		}
		loop {
			let Some(presp) = rx.recv().await else {
				// Fail-open is only safe while no body bytes have started flowing to ext_proc.
				if request_fsm
					.should_fail_open_on_disconnect(failure_mode, request_body_started_to_ext_proc)
				{
					trace!("fail open triggered");
					self.skipped = true;
					debug_assert_preserved_request_body(
						&req,
						had_body && rx_chunk.is_some(),
						"fail_open_on_disconnect_preserves_original_body_before_body_phase",
					);
					return Ok((req, None));
				}
				trace!("done receiving request");
				return Err(Error::NoMoreResponses);
			};
			if let Some(resp) = to_immediate_response(&presp) {
				trace!("got immediate response in request handler");
				return Ok((req, Some(resp)));
			}
			let step = self
				.process_request_loop_message(
					presp,
					Some(&mut req),
					&mut tx_chunk,
					&mut request_fsm,
					send_request_headers,
					request_body_streamed_before_header_response,
				)
				.await?;
			BufferedBodyPhase::update_deferred_mode(
				&mut pending_buffered_body,
				self.mode_state.request_body_mode,
			);
			BufferedBodyPhase::restore_original_if(
				&mut pending_buffered_body,
				request_fsm.should_restore_original_buffered_body(step.body_no_mutation),
				&mut tx_chunk,
			)
			.await;
			if step.transitioned {
				match request_fsm.body_path {
					BodyPath::Buffered | BodyPath::BufferedPartial => {
						// For buffered modes: ext_proc has finished with the headers. Buffer and
						// send the body now, after any headers mode_override has settled the
						// effective body mode.
						if pending_buffered_body.is_none() && rx_chunk.is_some() {
							let (req_with_channel, buffered_body) = start_buffered_request_body(
								req,
								self.mode_state.request_body_mode,
								had_body,
								&mut rx_chunk,
							);
							req = req_with_channel;
							pending_buffered_body = buffered_body;
						}
						let send_request_trailers =
							self.mode_state.request_trailer_mode == TrailerSendMode::Send;
						let sending_buffered_body = pending_buffered_body.is_some();
						match self
							.send_pending_buffered_body(
								&mut pending_buffered_body,
								&metadata_context,
								&mut first_message_when_headers_skipped,
								BodyStreamDirection::Request,
								send_request_trailers,
							)
							.await
						{
							Ok(true) => {
								request_body_started_to_ext_proc = true;
								// Loop again to receive the RequestBody response from ext_proc.
								continue;
							},
							Ok(false) => {},
							Err(e) => {
								if failure_mode == FailureMode::FailOpen
									&& !request_body_started_to_ext_proc
									&& !sending_buffered_body
								{
									trace!("fail open triggered");
									self.skipped = true;
									return Ok((req, None));
								}
								return Err(e);
							},
						}
					},
					BodyPath::FullDuplex => {
						if pending_full_duplex_body.is_none() {
							pending_full_duplex_body =
								BufferedBodyPhase::take_deferred_body(&mut pending_buffered_body);
							if pending_full_duplex_body.is_none() && rx_chunk.is_some() {
								let (req_with_channel, body) = attach_request_body_channel(req, &mut rx_chunk);
								req = req_with_channel;
								pending_full_duplex_body = Some(body);
							}
						}
					},
					BodyPath::None => {
						if let Some(original_body) =
							BufferedBodyPhase::take_deferred_body(&mut pending_buffered_body)
						{
							let (parts, _) = req.into_parts();
							let req = http::Request::from_parts(parts, original_body);
							debug_assert_preserved_request_body(
								&req,
								had_body,
								"body_mode_override_none_restores_original_body",
							);
							return Ok((req, None));
						}
						debug_assert_preserved_request_body(
							&req,
							had_body && rx_chunk.is_some(),
							"body_mode_override_none_preserves_original_body",
						);
						return Ok((req, None));
					},
				}

				if let Some(remainder) =
					BufferedBodyPhase::take_partial_remainder(&mut pending_buffered_body)
				{
					Self::spawn_forward_body_to_channel(
						remainder,
						tx_chunk.clone(),
						"failed to forward unprocessed request body remainder",
					);
				}

				if pending_full_duplex_body.is_some() && request_fsm.expect_body_response {
					let (first_message, sends_protocol_config) =
						FirstExtProcMessage::take_for_send(&mut first_message_when_headers_skipped);
					let send_request_trailers = self.mode_state.request_trailer_mode == TrailerSendMode::Send;
					self.spawn_body_stream(
						&metadata_context,
						&mut pending_full_duplex_body,
						tx.clone(),
						BodyStreamDirection::Request,
						send_request_trailers,
						first_message,
						"request body should be available before streaming continuation starts",
					);
					self.mark_protocol_config_sent_if(sends_protocol_config);
				}

				if !step.eos && request_fsm.body_path != BodyPath::BufferedPartial {
					request_fsm.enter_streaming_continuation();
					trace!("spawn body!");
					let immediate_response = self.request_body_immediate_response.clone();
					// Move remaining body response handling to an async task so we can return
					// the request to the caller while body chunks continue to flow.
					tokio::task::spawn(async move {
						loop {
							let Some(presp) = rx.recv().await else {
								trace!("done receiving request");
								return;
							};
							if let Some(resp) = to_immediate_response(&presp) {
								trace!("got immediate response during request body streaming");
								*immediate_response.lock().unwrap() = resp.direct_response;
								return;
							}
							let response = handle_response_for_request_mutation(
								request_fsm.expect_body_response,
								send_request_headers,
								false,
								None,
								&mut tx_chunk,
								presp,
							)
							.await;
							let (_, eos) = match response {
								Ok(response) => response,
								Err(error) => {
									warn!("request stream mutation failed: {error}");
									return;
								},
							};
							if eos || !request_fsm.expect_body_response {
								trace!("request EOS!");
								drop(tx_chunk);
								return;
							}
						}
					});
				}
				if request_fsm.expect_body_response
					&& (request_fsm
						.body_path
						.removes_content_length(send_request_headers)
						|| step.streamed_body_mutation)
				{
					req.headers_mut().remove(http::header::CONTENT_LENGTH);
				}
				return Ok((req, None));
			}
		}
	}

	async fn handle_body_stream(
		metadata_context: Option<Arc<Metadata>>,
		body: http::Body,
		tx: Sender<ProcessingRequest>,
		body_direction: BodyStreamDirection,
		send_trailers: bool,
		first_message: FirstExtProcMessage,
	) {
		if let Err(error) = Self::send_body_stream(
			metadata_context,
			body,
			tx,
			body_direction,
			send_trailers,
			first_message,
			"failed to read body stream",
		)
		.await
		{
			match error {
				Error::RequestSend => trace!("body stream stopped after ext_proc request channel closed"),
				error => warn!("body stream stopped: {error}"),
			}
		}
	}

	async fn send_body_stream(
		metadata_context: Option<Arc<Metadata>>,
		mut body: http::Body,
		tx: Sender<ProcessingRequest>,
		body_direction: BodyStreamDirection,
		send_trailers: bool,
		first_message: FirstExtProcMessage,
		body_error_message: &'static str,
	) -> Result<(), Error> {
		let mut first_message = first_message;
		let mut sent_end_stream = false;
		while let Some(frame) = body
			.frame()
			.await
			.transpose()
			// TODO(keithmattix): I don't love the use of the BodyBuffer error variant for this since we now
			// support buffering...
			.map_err(|e| Error::BodyBuffer(format!("{body_error_message}: {e}")))?
		{
			let request = Some(if frame.is_data() {
				let frame = frame.into_data().expect("already checked");
				let end_of_stream = body.is_end_stream();
				sent_end_stream |= end_of_stream;
				trace!("sending body chunk...",);
				body_direction.body_message(HttpBody {
					body: frame,
					end_of_stream,
				})
			} else if frame.is_trailers() {
				if !send_trailers {
					continue;
				}
				let frame = frame.into_trailers().expect("already checked");
				sent_end_stream = true;
				body_direction.trailers_message(HttpTrailers {
					trailers: to_header_map(&frame),
				})
			} else {
				// http_body::Frame only has data and trailers variants
				unreachable!("Frame is neither data nor trailers")
			});
			tx.send(ProcessingRequest {
				request,
				metadata_context: metadata_context.as_deref().cloned(),
				attributes: first_message.take_attributes_or_default(),
				protocol_config: first_message.take_protocol_config(),
				observability_mode: false,
			})
			.await
			.map_err(|_| Error::RequestSend)?;
		}

		if sent_end_stream {
			trace!("body request done");
			return Ok(());
		}

		// Send end of stream marker - try to unwrap Arc to avoid final clone
		let final_metadata = metadata_context.and_then(Arc::into_inner);
		tx.send(ProcessingRequest {
			request: Some(body_direction.body_message(HttpBody {
				body: Default::default(),
				end_of_stream: true,
			})),
			metadata_context: final_metadata,
			attributes: first_message.take_attributes_or_default(),
			protocol_config: first_message.take_protocol_config(),
			observability_mode: false,
		})
		.await
		.map_err(|_| Error::RequestSend)?;
		trace!("body request done");
		Ok(())
	}

	async fn forward_body_to_channel(
		mut body: http::Body,
		tx_chunk: Sender<Result<Frame<Bytes>, Infallible>>,
		body_error_message: &'static str,
	) -> Result<(), Error> {
		while let Some(frame) = body
			.frame()
			.await
			.transpose()
			.map_err(|e| Error::BodyBuffer(format!("{body_error_message}: {e}")))?
		{
			tx_chunk
				.send(Ok(frame))
				.await
				.map_err(|_| Error::RequestSend)?;
		}
		Ok(())
	}

	fn spawn_forward_body_to_channel(
		body: http::Body,
		tx_chunk: Sender<Result<Frame<Bytes>, Infallible>>,
		body_error_message: &'static str,
	) {
		tokio::task::spawn(async move {
			if let Err(error) = Self::forward_body_to_channel(body, tx_chunk, body_error_message).await {
				match error {
					Error::RequestSend => trace!("body remainder stopped after body channel closed"),
					error => warn!("body remainder stopped: {error}"),
				}
			}
		});
	}

	pub async fn mutate_response(
		&mut self,
		response: http::Response,
		request: Option<&RequestSnapshot>,
		resolved_destination_metadata: Option<SocketAddr>,
	) -> Result<(http::Response, Option<PolicyResponse>), Error> {
		if self.skipped {
			return Ok((response, None));
		}
		let headers = resp_to_header_map(&response);
		let send_response_headers = self.mode_state.response_header_mode == HeaderSendMode::Send;

		let exec = cel::Executor::new_response(request, &response);
		// Wrap metadata_context in Arc for cheap cloning across body chunks
		let metadata_context = if self.metadata_context.is_none()
			&& let Some(rd) = resolved_destination_metadata
		{
			Some(Arc::new(Metadata {
				filter_metadata: HashMap::from([(
					// This is gross, but the GIE project unfairly favors Envoy, so we have to adapt to its limitations.
					"envoy.lb".to_string(),
					serde_json::from_value(serde_json::json!({"x-gateway-destination-endpoint-served": rd}))
						.unwrap(),
				)]),
			}))
		} else {
			self.metadata_context.as_ref().map(|meta| {
				Arc::new(Metadata {
					filter_metadata: meta
						.iter()
						.filter_map(|(n, e)| eval_to_struct(&exec, e).map(|v| (n.clone(), v)).ok())
						.collect(),
				})
			})
		};
		// response_attributes should only be sent on first ProcessingRequest
		// this will need to be modified if we configure which Requests to send
		let attributes = self
			.resp_attributes
			.as_ref()
			.and_then(|attrs| {
				eval_to_struct(&exec, attrs)
					.map(|v| HashMap::from([(EXTPROC_ATTRIBUTES_NAMESPACE.to_string(), v)]))
					.ok()
			})
			.unwrap_or_default();
		let max_response_bytes = http::response_buffer_limit(&response);
		let (parts, body) = response.into_parts();
		let end_of_stream = body.is_end_stream();
		let had_body = !end_of_stream;
		let response_body_mode = self.mode_state.response_body_mode;
		// response_fsm drives local flow in mutate_response, while mode_state tracks
		// the currently effective ext_proc processing modes.
		let mut response_fsm =
			ResponseFlowFsm::new(send_response_headers, response_body_mode, had_body);
		let protocol_config = self.protocol_config();
		let mut first_message = FirstExtProcMessage::for_body_phase(
			send_response_headers,
			response_fsm.send_body,
			attributes.clone(),
			protocol_config,
			self.protocol_config_sent,
		);

		// Send the response headers to ext_proc.
		// No response side fail_open handling.
		if send_response_headers {
			let (header_protocol_config, sends_protocol_config) =
				self.protocol_config_for_headers(protocol_config);
			self
				.send_request(ProcessingRequest {
					request: Some(Request::ResponseHeaders(HttpHeaders {
						headers,
						end_of_stream,
					})),
					metadata_context: metadata_context.as_deref().cloned(),
					attributes: attributes.clone(),
					protocol_config: header_protocol_config,
					observability_mode: false,
				})
				.await?;
			self.mark_protocol_config_sent_if(sends_protocol_config);
		}

		if response_fsm.phase == ResponsePhase::Complete {
			return Ok((http::Response::from_parts(parts, body), None));
		}

		let tx = self.tx_req.clone();
		let mut pending_response_body = Some(body);
		let mut pending_response_buffer = None;
		// Now we need to build the new body. This is going to be streamed in from the ext_proc server.
		let (mut tx_chunk, rx_chunk) = tokio::sync::mpsc::channel(1);
		let body = http_body_util::StreamBody::new(ReceiverStream::new(rx_chunk));
		let mut resp = http::Response::from_parts(parts, http::Body::new(body));

		// FULL_DUPLEX_STREAMED sends response body chunks as they arrive. The ext_proc server may
		// buffer the response headers and complete body before sending any response, so do not wait
		// for the response-headers ProcessingResponse before forwarding body frames to ext_proc.
		let response_body_streamed_before_header_response =
			response_body_mode == BodySendMode::FullDuplexStreamed && had_body;
		if response_body_streamed_before_header_response {
			let (first_message, sends_protocol_config) =
				FirstExtProcMessage::take_for_send(&mut first_message);
			let send_response_trailers = self.mode_state.response_trailer_mode == TrailerSendMode::Send;
			self.spawn_body_stream(
				&metadata_context,
				&mut pending_response_body,
				tx.clone(),
				BodyStreamDirection::Response,
				send_response_trailers,
				first_message,
				"response body should be available before streaming starts",
			);
			self.mark_protocol_config_sent_if(sends_protocol_config);
		} else if !send_response_headers && had_body {
			pending_response_buffer = start_buffered_response_body(
				&mut pending_response_body,
				response_body_mode,
				had_body,
				max_response_bytes,
			);
			let send_response_trailers = self.mode_state.response_trailer_mode == TrailerSendMode::Send;
			let sent_buffered = self
				.send_pending_buffered_body(
					&mut pending_response_buffer,
					&metadata_context,
					&mut first_message,
					BodyStreamDirection::Response,
					send_response_trailers,
				)
				.await?;
			if !sent_buffered {
				let (first_message, sends_protocol_config) =
					FirstExtProcMessage::take_for_send(&mut first_message);
				let send_response_trailers = self.mode_state.response_trailer_mode == TrailerSendMode::Send;
				self.spawn_body_stream(
					&metadata_context,
					&mut pending_response_body,
					tx.clone(),
					BodyStreamDirection::Response,
					send_response_trailers,
					first_message,
					"response body should be available before streaming starts",
				);
				self.mark_protocol_config_sent_if(sends_protocol_config);
			}
		}

		let mut rx = self
			.rx_resp_for_response
			.take()
			.expect("mutate_response called twice");
		loop {
			let msg = Self::recv_response_loop_message(&mut rx).await?;
			let (transitioned, eos, streamed_body_mutation) = match msg {
				ResponseLoopMessage::Immediate(dr) => return Ok((resp, Some(dr))),
				ResponseLoopMessage::Processing(presp) => {
					let response_body_no_mutation = response_body_response_has_no_mutation(&presp);
					let streamed_body_mutation = response_response_has_streamed_body_mutation(&presp);
					let result = self
						.process_response_loop_message(
							presp,
							Some(&mut resp),
							&mut tx_chunk,
							&mut response_fsm,
							send_response_headers,
							response_body_streamed_before_header_response,
						)
						.await?;
					BufferedBodyPhase::update_deferred_mode(
						&mut pending_response_buffer,
						self.mode_state.response_body_mode,
					);
					// For buffered modes: if ext_proc returned no body mutation, forward the
					// original buffered bytes to the client instead of an empty body.
					BufferedBodyPhase::restore_original_if(
						&mut pending_response_buffer,
						response_fsm.should_restore_original_buffered_body(response_body_no_mutation),
						&mut tx_chunk,
					)
					.await;
					(result.0, result.1, streamed_body_mutation)
				},
			};
			if transitioned {
				match response_fsm.body_path {
					BodyPath::Buffered | BodyPath::BufferedPartial if !eos && response_fsm.send_body => {
						if pending_response_buffer.is_none() {
							pending_response_buffer = start_buffered_response_body(
								&mut pending_response_body,
								self.mode_state.response_body_mode,
								had_body,
								max_response_bytes,
							);
						}
						let send_response_trailers =
							self.mode_state.response_trailer_mode == TrailerSendMode::Send;
						if self
							.send_pending_buffered_body(
								&mut pending_response_buffer,
								&metadata_context,
								&mut first_message,
								BodyStreamDirection::Response,
								send_response_trailers,
							)
							.await?
						{
							continue;
						}
					},
					BodyPath::FullDuplex
						if !eos && response_fsm.send_body && pending_response_body.is_none() =>
					{
						pending_response_body =
							BufferedBodyPhase::take_deferred_body(&mut pending_response_buffer);
					},
					BodyPath::None => {
						if let Some(original_body) =
							BufferedBodyPhase::take_deferred_body(&mut pending_response_buffer)
						{
							let (parts, _) = resp.into_parts();
							return Ok((http::Response::from_parts(parts, original_body), None));
						}
						if let Some(original_body) = pending_response_body.take() {
							let (parts, _) = resp.into_parts();
							return Ok((http::Response::from_parts(parts, original_body), None));
						}
					},
					_ => {},
				}
				if let Some(remainder) =
					BufferedBodyPhase::take_partial_remainder(&mut pending_response_buffer)
				{
					Self::spawn_forward_body_to_channel(
						remainder,
						tx_chunk.clone(),
						"failed to forward unprocessed response body remainder",
					);
				}
				if !eos && response_fsm.send_body && response_fsm.body_path == BodyPath::FullDuplex {
					if pending_response_body.is_some() {
						let (first_message, sends_protocol_config) =
							FirstExtProcMessage::take_for_send(&mut first_message);
						trace!("spawn body!");
						let send_response_trailers =
							self.mode_state.response_trailer_mode == TrailerSendMode::Send;
						self.spawn_body_stream(
							&metadata_context,
							&mut pending_response_body,
							tx.clone(),
							BodyStreamDirection::Response,
							send_response_trailers,
							first_message,
							"response body should be available before streaming continuation starts",
						);
						self.mark_protocol_config_sent_if(sends_protocol_config);
					}
					response_fsm.enter_streaming_continuation();
					tokio::task::spawn(Self::forward_response_stream_continuation(
						rx,
						tx_chunk,
						true,
						send_response_headers,
					));
				}
				if response_fsm.send_body
					&& (response_fsm
						.body_path
						.removes_content_length(send_response_headers)
						|| streamed_body_mutation)
				{
					resp.headers_mut().remove(http::header::CONTENT_LENGTH);
				}
				return Ok((resp, None));
			}
		}
	}
}

fn eval_expression(exec: &Executor, v: &Expression) -> Result<prost_wkt_types::Value, ProxyError> {
	let res = exec.eval(v).map_err(|e| ProxyError::Processing(e.into()))?;
	let js = res
		.json()
		.map_err(|_| ProxyError::Processing(cel::Error::JsonConvert.into()))?;
	envoy_proto_common::json_to_prost_value(js)
}

fn eval_to_struct(
	exec: &Executor<'_>,
	expressions: &HashMap<String, Arc<cel::Expression>>,
) -> Result<prost_wkt_types::Struct, ProxyError> {
	Ok(Struct {
		fields: expressions
			.iter()
			.filter_map(|(key, expr)| match eval_expression(exec, expr) {
				Ok(result) => Some((key.clone(), result)),
				Err(error) => {
					warn!(%key, %error, "failed to evaluate metadata_context CEL expression");
					None
				},
			})
			.collect(),
	})
}
