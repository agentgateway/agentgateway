use std::collections::HashMap;

use prost_wkt_types::Struct;
use protos::envoy::service::ext_proc::v3::{
	BodySendMode as EnvoyBodySendMode, ProtocolConfiguration,
};

use super::proto;
use super::proto::processing_request::Request;
use crate::http::PolicyResponse;
use crate::http::ext_proc::proto::{HttpBody, HttpTrailers, ProcessingResponse};
use crate::*;

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "ExtProcFailureMode"))]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	/// Reject the request when the external processing service fails.
	#[default]
	FailClosed,
	/// Continue the request when the external processing service fails.
	FailOpen,
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum BodySendMode {
	/// Do not send the body to the external processing service.
	None,
	/// Buffer and send the full body to the external processing service.
	Buffered,
	/// Buffer and send the body up to the configured limit.
	BufferedPartial,
	/// Stream the body bidirectionally with the external processing service.
	#[default]
	FullDuplexStreamed,
}

impl From<BodySendMode> for EnvoyBodySendMode {
	fn from(val: BodySendMode) -> Self {
		match val {
			BodySendMode::None => EnvoyBodySendMode::None,
			BodySendMode::Buffered => EnvoyBodySendMode::Buffered,
			BodySendMode::BufferedPartial => EnvoyBodySendMode::BufferedPartial,
			BodySendMode::FullDuplexStreamed => EnvoyBodySendMode::FullDuplexStreamed,
		}
	}
}

impl From<BodySendMode> for i32 {
	fn from(val: BodySendMode) -> Self {
		match val {
			BodySendMode::None => EnvoyBodySendMode::None as i32,
			BodySendMode::Buffered => EnvoyBodySendMode::Buffered as i32,
			BodySendMode::BufferedPartial => EnvoyBodySendMode::BufferedPartial as i32,
			BodySendMode::FullDuplexStreamed => EnvoyBodySendMode::FullDuplexStreamed as i32,
		}
	}
}

impl From<EnvoyBodySendMode> for BodySendMode {
	fn from(val: EnvoyBodySendMode) -> Self {
		match val {
			EnvoyBodySendMode::None => BodySendMode::None,
			EnvoyBodySendMode::Buffered => BodySendMode::Buffered,
			EnvoyBodySendMode::BufferedPartial => BodySendMode::BufferedPartial,
			EnvoyBodySendMode::FullDuplexStreamed => BodySendMode::FullDuplexStreamed,
			EnvoyBodySendMode::Streamed => {
				// This mode is not currently supported, so we map it to the closest available mode and log a warning.
				warn!(
					"received unsupported Envoy body send mode STREAMED; treating as FULL_DUPLEX_STREAMED"
				);
				BodySendMode::FullDuplexStreamed
			},
		}
	}
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum HeaderSendMode {
	/// Send headers to the external processing service.
	#[default]
	Send,
	/// Do not send headers to the external processing service.
	Skip,
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum TrailerSendMode {
	/// Send trailers to the external processing service.
	#[default]
	Send,
	/// Do not send trailers to the external processing service.
	Skip,
}

#[apply(schema!)]
#[derive(Copy)]
pub struct ProcessingOptions {
	/// How request bodies are sent to the external processing service.
	#[serde(default = "default_body_send_mode")]
	pub request_body_mode: BodySendMode,
	/// How response bodies are sent to the external processing service.
	#[serde(default = "default_body_send_mode")]
	pub response_body_mode: BodySendMode,
	/// Whether request headers are sent to the external processing service.
	#[serde(default)]
	pub request_header_mode: HeaderSendMode,
	/// Whether response headers are sent to the external processing service.
	#[serde(default)]
	pub response_header_mode: HeaderSendMode,
	/// Whether request trailers are sent to the external processing service.
	#[serde(default)]
	pub request_trailer_mode: TrailerSendMode,
	/// Whether response trailers are sent to the external processing service.
	#[serde(default)]
	pub response_trailer_mode: TrailerSendMode,
	/// Whether the external processing service can change processing modes during a request.
	#[serde(default)]
	pub allow_mode_override: bool,
}

fn default_body_send_mode() -> BodySendMode {
	BodySendMode::FullDuplexStreamed
}

impl Default for ProcessingOptions {
	fn default() -> Self {
		Self {
			request_body_mode: BodySendMode::FullDuplexStreamed,
			response_body_mode: BodySendMode::FullDuplexStreamed,
			request_header_mode: HeaderSendMode::Send,
			response_header_mode: HeaderSendMode::Send,
			request_trailer_mode: TrailerSendMode::Send,
			response_trailer_mode: TrailerSendMode::Send,
			allow_mode_override: false,
		}
	}
}

#[derive(Debug, Copy, Clone)]
pub(super) enum HeaderPhase {
	Request,
	Response,
}

#[derive(Debug, Copy, Clone)]
enum BodyModeOverrideDirection {
	Request,
	Response,
}

// Tracks the negotiated ext_proc processing modes for the lifetime of a single
// gRPC bidirectional stream. This is the protocol-facing state: it stores what we believe
// Envoy ext_proc wants us to send next, and applies mode_override updates with
// protocol validity checks.
#[derive(Debug, Copy, Clone)]
pub(super) struct ModeStateMachine {
	pub(super) request_body_mode: BodySendMode,
	pub(super) response_body_mode: BodySendMode,
	pub(super) request_header_mode: HeaderSendMode,
	pub(super) response_header_mode: HeaderSendMode,
	pub(super) request_trailer_mode: TrailerSendMode,
	pub(super) response_trailer_mode: TrailerSendMode,
	pub(super) allow_mode_override: bool,
	request_headers_processed: bool,
	response_headers_processed: bool,
}

impl From<ProcessingOptions> for ModeStateMachine {
	fn from(opts: ProcessingOptions) -> Self {
		let mut state = Self {
			request_body_mode: opts.request_body_mode,
			response_body_mode: opts.response_body_mode,
			request_header_mode: opts.request_header_mode,
			response_header_mode: opts.response_header_mode,
			request_trailer_mode: opts.request_trailer_mode,
			response_trailer_mode: opts.response_trailer_mode,
			allow_mode_override: opts.allow_mode_override,
			request_headers_processed: false,
			response_headers_processed: false,
		};
		state.enforce_full_duplex_trailer_modes();
		state
	}
}

impl ModeStateMachine {
	fn enforce_full_duplex_trailer_modes(&mut self) {
		let is_full_duplex = self.request_body_mode == BodySendMode::FullDuplexStreamed
			|| self.response_body_mode == BodySendMode::FullDuplexStreamed;
		let trailers_incompatible = self.request_trailer_mode == TrailerSendMode::Skip
			|| self.response_trailer_mode == TrailerSendMode::Skip;
		if is_full_duplex && trailers_incompatible {
			warn!(
				request_body_mode = ?self.request_body_mode,
				response_body_mode = ?self.response_body_mode,
				request_trailer_mode = ?self.request_trailer_mode,
				response_trailer_mode = ?self.response_trailer_mode,
				"FULL_DUPLEX_STREAMED body mode is incompatible with trailers modes of SKIP; overriding trailers modes to SEND"
			);
			self.request_trailer_mode = TrailerSendMode::Send;
			self.response_trailer_mode = TrailerSendMode::Send;
		}
	}
	fn current_mode_allows_body_override(
		current_mode: BodySendMode,
		phase: HeaderPhase,
		direction: BodyModeOverrideDirection,
	) -> bool {
		if current_mode == BodySendMode::FullDuplexStreamed {
			warn!(
				phase = ?phase,
				direction = ?direction,
				"ignoring body mode_override because current body mode is FULL_DUPLEX_STREAMED"
			);
			return false;
		}
		true
	}

	fn apply_body_mode_override(
		current_mode: &mut BodySendMode,
		next_mode: EnvoyBodySendMode,
		phase: HeaderPhase,
		direction: BodyModeOverrideDirection,
		phase_name: &'static str,
		field_name: &'static str,
	) {
		match next_mode {
			// TODO: Should we abort the stream?
			EnvoyBodySendMode::Streamed => {
				warn!(
					phase = phase_name,
					field = field_name,
					"mode_override body_mode=STREAMED is not implemented; keeping current body mode"
				)
			},
			_ if Self::current_mode_allows_body_override(*current_mode, phase, direction) => {
				*current_mode = BodySendMode::from(next_mode);
			},
			_ => {},
		}
	}

	pub(super) fn mark_headers_processed(&mut self, phase: HeaderPhase) {
		match phase {
			HeaderPhase::Request => self.request_headers_processed = true,
			HeaderPhase::Response => self.response_headers_processed = true,
		}
	}

	pub(super) fn apply_envoy_mode_override(
		&mut self,
		phase: HeaderPhase,
		mode: &proto::ProcessingMode,
	) {
		use proto::processing_mode::HeaderSendMode as EnvoyHeaderSendMode;
		let phase_name = match phase {
			HeaderPhase::Request => "request_headers",
			HeaderPhase::Response => "response_headers",
		};

		if mode.request_header_mode != EnvoyHeaderSendMode::Default as i32 {
			warn!(
				phase = phase_name,
				"mode_override.request_header_mode is ignored by protocol"
			);
		}

		if let Ok(hm) = EnvoyHeaderSendMode::try_from(mode.response_header_mode) {
			match hm {
				EnvoyHeaderSendMode::Default => {},
				EnvoyHeaderSendMode::Send if !self.response_headers_processed => {
					self.response_header_mode = HeaderSendMode::Send;
				},
				EnvoyHeaderSendMode::Skip if !self.response_headers_processed => {
					self.response_header_mode = HeaderSendMode::Skip;
				},
				_ => {},
			}
		}

		if let Ok(bm) = EnvoyBodySendMode::try_from(mode.request_body_mode) {
			Self::apply_body_mode_override(
				&mut self.request_body_mode,
				bm,
				phase,
				BodyModeOverrideDirection::Request,
				phase_name,
				"request_body_mode",
			);
		}

		if let Ok(bm) = EnvoyBodySendMode::try_from(mode.response_body_mode) {
			Self::apply_body_mode_override(
				&mut self.response_body_mode,
				bm,
				phase,
				BodyModeOverrideDirection::Response,
				phase_name,
				"response_body_mode",
			);
		}

		if let Ok(hm) = EnvoyHeaderSendMode::try_from(mode.request_trailer_mode) {
			match hm {
				EnvoyHeaderSendMode::Default => {},
				EnvoyHeaderSendMode::Send => self.request_trailer_mode = TrailerSendMode::Send,
				EnvoyHeaderSendMode::Skip => self.request_trailer_mode = TrailerSendMode::Skip,
			}
		}
		if let Ok(hm) = EnvoyHeaderSendMode::try_from(mode.response_trailer_mode) {
			match hm {
				EnvoyHeaderSendMode::Default => {},
				EnvoyHeaderSendMode::Send => self.response_trailer_mode = TrailerSendMode::Send,
				EnvoyHeaderSendMode::Skip => self.response_trailer_mode = TrailerSendMode::Skip,
			}
		}
		self.enforce_full_duplex_trailer_modes();
	}
}

// Request-side execution FSM. Unlike ModeStateMachine (which stores ext_proc
// protocol configuration), this FSM captures local control-flow transitions in
// mutate_request: whether we are waiting on headers, body, or can return.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum RequestPhase {
	AwaitingHeaders,
	AwaitingBody,
	StreamingContinuation,
	Complete,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum BodyPath {
	None,
	Buffered,
	BufferedPartial,
	FullDuplex,
}

#[derive(Debug, Copy, Clone)]
pub(super) struct RequestFlowFsm {
	pub(super) phase: RequestPhase,
	pub(super) body_path: BodyPath,
	had_body: bool,
	pub(super) expect_body_response: bool,
}

impl RequestFlowFsm {
	pub(super) fn new(send_headers: bool, had_body: bool, body_mode: BodySendMode) -> Self {
		let body_path = BodyPath::from(body_mode);
		let expect_body_response = body_path != BodyPath::None && had_body;
		let phase = if send_headers {
			RequestPhase::AwaitingHeaders
		} else if expect_body_response {
			RequestPhase::AwaitingBody
		} else {
			RequestPhase::Complete
		};
		Self {
			phase,
			body_path,
			had_body,
			expect_body_response,
		}
	}

	pub(super) fn reconcile_potential_mode_override(&mut self, mode: BodySendMode) {
		self.body_path = BodyPath::from(mode);
		self.expect_body_response = self.body_path != BodyPath::None && self.had_body;
		if self.phase == RequestPhase::AwaitingBody && !self.expect_body_response {
			self.phase = RequestPhase::Complete;
		}
	}

	fn finish_headers_phase(&mut self) {
		self.phase = if self.expect_body_response {
			RequestPhase::AwaitingBody
		} else {
			RequestPhase::Complete
		};
	}

	pub(super) fn enter_streaming_continuation(&mut self) {
		self.phase = RequestPhase::StreamingContinuation;
	}

	pub(super) fn advance_after_response(&mut self, headers_done: bool) -> bool {
		if headers_done {
			self.finish_headers_phase();
		}
		self.phase != RequestPhase::AwaitingHeaders
	}

	pub(super) fn should_restore_original_buffered_body(&self, body_no_mutation: bool) -> bool {
		body_no_mutation && self.expect_body_response && self.body_path != BodyPath::FullDuplex
	}

	pub(super) fn should_fail_open_on_disconnect(
		&self,
		failure_mode: FailureMode,
		body_started_to_ext_proc: bool,
	) -> bool {
		failure_mode == FailureMode::FailOpen && !body_started_to_ext_proc
	}
}

impl From<BodySendMode> for BodyPath {
	fn from(mode: BodySendMode) -> Self {
		match mode {
			BodySendMode::None => BodyPath::None,
			BodySendMode::Buffered => BodyPath::Buffered,
			BodySendMode::BufferedPartial => BodyPath::BufferedPartial,
			BodySendMode::FullDuplexStreamed => BodyPath::FullDuplex,
		}
	}
}

impl BodyPath {
	pub(super) fn removes_content_length(self, send_headers: bool) -> bool {
		match self {
			BodyPath::None => false,
			BodyPath::Buffered => !send_headers,
			BodyPath::BufferedPartial | BodyPath::FullDuplex => true,
		}
	}

	pub(super) fn validates_content_length(self, send_headers: bool) -> bool {
		self == BodyPath::Buffered && send_headers
	}
}

// Response-side execution FSM mirroring RequestFlowFsm. This controls local
// phase progression in mutate_response independently of protocol mode storage.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(super) enum ResponsePhase {
	AwaitingHeaders,
	AwaitingBody,
	StreamingContinuation,
	Complete,
}

#[derive(Debug, Copy, Clone)]
pub(super) struct ResponseFlowFsm {
	pub(super) phase: ResponsePhase,
	pub(super) body_path: BodyPath,
	had_body: bool,
	pub(super) send_body: bool,
}

pub(super) enum ResponseLoopMessage {
	Immediate(PolicyResponse),
	Processing(ProcessingResponse),
}

pub(super) struct RequestLoopStep {
	pub(super) transitioned: bool,
	pub(super) eos: bool,
	pub(super) body_no_mutation: bool,
	pub(super) streamed_body_mutation: bool,
}

#[derive(Copy, Clone)]
pub(super) enum BodyStreamDirection {
	Request,
	Response,
}

impl BodyStreamDirection {
	pub(super) fn body_message(self, body: HttpBody) -> Request {
		match self {
			Self::Request => Request::RequestBody(body),
			Self::Response => Request::ResponseBody(body),
		}
	}

	pub(super) fn trailers_message(self, trailers: HttpTrailers) -> Request {
		match self {
			Self::Request => Request::RequestTrailers(trailers),
			Self::Response => Request::ResponseTrailers(trailers),
		}
	}
}

impl ResponseFlowFsm {
	pub(super) fn new(send_headers: bool, body_mode: BodySendMode, had_body: bool) -> Self {
		let body_path = BodyPath::from(body_mode);
		let send_body = body_path != BodyPath::None && had_body;
		let phase = if send_headers {
			ResponsePhase::AwaitingHeaders
		} else if send_body {
			ResponsePhase::AwaitingBody
		} else {
			ResponsePhase::Complete
		};
		Self {
			phase,
			body_path,
			had_body,
			send_body,
		}
	}

	pub(super) fn reconcile_potential_mode_override(&mut self, mode: BodySendMode) {
		self.body_path = BodyPath::from(mode);
		self.send_body = self.body_path != BodyPath::None && self.had_body;
		if self.phase == ResponsePhase::AwaitingBody && !self.send_body {
			self.phase = ResponsePhase::Complete;
		}
	}

	fn finish_headers_phase(&mut self) {
		self.phase = if self.send_body {
			ResponsePhase::AwaitingBody
		} else {
			ResponsePhase::Complete
		};
	}

	pub(super) fn enter_streaming_continuation(&mut self) {
		self.phase = ResponsePhase::StreamingContinuation;
	}

	pub(super) fn advance_after_response(&mut self, headers_done: bool) -> bool {
		if headers_done {
			self.finish_headers_phase();
		}
		self.phase != ResponsePhase::AwaitingHeaders
	}

	pub(super) fn should_restore_original_buffered_body(&self, body_no_mutation: bool) -> bool {
		body_no_mutation && self.send_body && self.body_path != BodyPath::FullDuplex
	}
}

#[derive(Debug, Default)]
pub(super) struct FirstExtProcMessage {
	attributes: Option<HashMap<String, Struct>>,
	protocol_config: Option<ProtocolConfiguration>,
}

impl FirstExtProcMessage {
	pub(super) fn for_body_phase(
		send_headers: bool,
		send_body: bool,
		attributes: HashMap<String, Struct>,
		protocol_config: ProtocolConfiguration,
		protocol_config_sent: bool,
	) -> Self {
		Self {
			attributes: (!send_headers && send_body).then_some(attributes),
			protocol_config: (!send_headers && send_body && !protocol_config_sent)
				.then_some(protocol_config),
		}
	}

	pub(super) fn take_attributes_or_default(&mut self) -> HashMap<String, Struct> {
		self.attributes.take().unwrap_or_default()
	}

	pub(super) fn take_protocol_config(&mut self) -> Option<ProtocolConfiguration> {
		self.protocol_config.take()
	}

	fn has_protocol_config(&self) -> bool {
		self.protocol_config.is_some()
	}

	pub(super) fn take_for_send(message: &mut Self) -> (Self, bool) {
		let message = std::mem::take(message);
		let sends_protocol_config = message.has_protocol_config();
		(message, sends_protocol_config)
	}
}
