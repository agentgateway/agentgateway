//! Dispatch and notification routing for the MCP relay.
//!
//! This module contains session-wide orchestration over upstream targets. It
//! does not own target-local protocol state; `TargetSession` remains the owner
//! of per-target lifecycle bookkeeping.

use super::*;
use crate::mcp::mergestream::{MergeFn, Messages};
use futures_core::Stream;
use futures_util::{StreamExt, future::join_all};

#[derive(Debug, Clone, Copy)]
enum FanoutMode {
	All,
	Targeted,
}

impl FanoutMode {
	fn stream_failure_log(self) -> &'static str {
		match self {
			FanoutMode::All => "upstream failed during fanout; excluding from results",
			FanoutMode::Targeted => "targeted upstream failed during fanout; excluding",
		}
	}
}

impl Relay {
	async fn build_fanout_streams(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
		names: Vec<Strng>,
		mode: FanoutMode,
	) -> Result<Vec<(Strng, mergestream::Messages)>, UpstreamError> {
		let ctx = ctx.clone();
		let futures = names
			.into_iter()
			.map(|name| {
				let con = self
					.upstreams
					.get(name.as_ref())
					.map_err(|e| UpstreamError::InvalidRequest(e.to_string()))?;
				let request = request.clone();
				let ctx = ctx.clone();
				Ok(async move { (name, con.generic_stream(request, &ctx).await) })
			})
			.collect::<Result<Vec<_>, UpstreamError>>()?;
		let mut streams = Vec::with_capacity(futures.len());
		for (name, result) in join_all(futures).await {
			match result {
				Ok(stream) => streams.push((name, stream)),
				Err(e) => {
					if !self.allow_degraded {
						return Err(e);
					}
					tracing::warn!(%name, ?e, "{}", mode.stream_failure_log());
				},
			}
		}
		Ok(streams)
	}

	async fn prepare_selected_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		names: Vec<Strng>,
		fanout_mode: FanoutMode,
		empty_error: String,
	) -> Result<(RequestId, mergestream::MergeStream), UpstreamError> {
		let id = r.id.clone();
		let streams = self
			.build_fanout_streams(r, &ctx, names, fanout_mode)
			.await?;

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(empty_error));
		}

		let ms = mergestream::MergeStream::new(
			streams,
			id.clone(),
			merge,
			!self.allow_degraded,
			self.message_mapper(),
		);
		Ok((id, ms))
	}

	pub async fn send_single(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		service_name: &str,
	) -> Result<Response, UpstreamError> {
		let id = r.id.clone();
		let Ok(us) = self.upstreams.get(service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		let relay = self.clone();
		let server_name = service_name.to_string();
		let stream = us
			.generic_stream(r, &ctx)
			.await?
			.map(move |msg| msg.and_then(|msg| relay.map_server_message(&server_name, msg)));

		messages_to_response(id, stream)
	}

	pub async fn send_fanout_deletion(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		for (name, con) in self.upstreams.iter_named() {
			if let Err(e) = con.delete(&ctx).await {
				if !self.allow_degraded {
					return Err(e);
				}
				tracing::warn!(%name, ?e, "upstream failed during session deletion; continuing cleanup");
			}
		}
		Ok(accepted_response())
	}

	pub async fn get_event_stream_messages(
		&self,
		ctx: IncomingRequestContext,
	) -> Result<Messages, UpstreamError> {
		let mut streams = Vec::new();
		for (name, con) in self.upstreams.iter_named() {
			if self.allow_degraded {
				match con.get_event_stream(&ctx).await {
					Ok(s) => streams.push((name, s)),
					Err(e) => {
						tracing::debug!(
							%name,
							?e,
							"upstream failed to provide event stream; skipping for this session"
						);
					},
				}
			} else {
				streams.push((name, con.get_event_stream(&ctx).await?));
			}
		}

		if streams.is_empty() {
			return Err(UpstreamError::InvalidRequest(
				"all upstreams failed to provide event stream".to_string(),
			));
		}

		let ms = mergestream::MergeStream::new_without_merge(streams, self.message_mapper());
		Ok(Messages::from_stream(ms))
	}

	pub async fn send_fanout(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
	) -> Result<Response, UpstreamError> {
		let names = self
			.upstreams
			.iter_named()
			.map(|(name, _)| name)
			.collect::<Vec<_>>();
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				merge,
				names,
				FanoutMode::All,
				"all upstreams failed to respond to fanout".to_string(),
			)
			.await?;
		messages_to_response(id, stream)
	}

	pub async fn send_initialize(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		pv: ProtocolVersion,
	) -> Result<Response, UpstreamError> {
		let names = self
			.upstreams
			.iter_named()
			.map(|(name, _)| name)
			.collect::<Vec<_>>();
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				self.merge_initialize(pv),
				names,
				FanoutMode::All,
				"all upstreams failed to respond to fanout".to_string(),
			)
			.await?;
		messages_to_buffered_response(id, stream).await
	}

	pub async fn send_fanout_to(
		&self,
		r: JsonRpcRequest<ClientRequest>,
		ctx: IncomingRequestContext,
		merge: Box<MergeFn>,
		names: Vec<Strng>,
	) -> Result<Response, UpstreamError> {
		let method = r.request.method().to_string();
		if names.is_empty() {
			return Err(UpstreamError::InvalidMethod(format!(
				"no eligible backends for method {method}",
			)));
		}
		let (id, stream) = self
			.prepare_selected_fanout(
				r,
				ctx,
				merge,
				names,
				FanoutMode::Targeted,
				format!("all eligible backends failed for method {method}"),
			)
			.await?;
		messages_to_response(id, stream)
	}

	pub async fn send_notification(
		&self,
		r: JsonRpcNotification<ClientNotification>,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		match r.notification {
			ClientNotification::CancelledNotification(mut cn) => {
				if let Ok((server_name, original_id)) =
					self.decode_upstream_request_id(&cn.params.request_id)
				{
					if let Ok(us) = self.upstreams.get(&server_name) {
						cn.params.request_id = original_id;
						if let Err(e) = us
							.generic_notification(ClientNotification::CancelledNotification(cn), &ctx)
							.await
						{
							if !self.allow_degraded {
								return Err(e);
							}
							tracing::warn!(%server_name, ?e, "targeted cancellation failed");
						}
						return Ok(accepted_response());
					}
					tracing::warn!(
						%server_name,
						request_id = %cn.params.request_id,
						"dropping cancellation: upstream not found"
					);
					return Ok(accepted_response());
				}
				if self.uses_routed_identifiers() {
					tracing::warn!(
						request_id = %cn.params.request_id,
						"dropping cancellation: failed to decode routed request id"
					);
					return Ok(accepted_response());
				}
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(ClientNotification::CancelledNotification(cn.clone()), &ctx)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "cancellation fanout failed; ignoring");
					}
				}
			},
			ClientNotification::ProgressNotification(mut pn) => {
				if let Ok((server_name, original_progress_token)) =
					self.decode_upstream_progress_token(&pn.params.progress_token)
				{
					if let Ok(us) = self.upstreams.get(&server_name) {
						pn.params.progress_token = original_progress_token;
						if let Err(e) = us
							.generic_notification(ClientNotification::ProgressNotification(pn), &ctx)
							.await
						{
							if !self.allow_degraded {
								return Err(e);
							}
							tracing::warn!(%server_name, ?e, "targeted progress notification failed");
						}
						return Ok(accepted_response());
					}
					tracing::warn!(
						%server_name,
						progress_token = %pn.params.progress_token.0,
						"dropping progress notification: upstream not found"
					);
					return Ok(accepted_response());
				}
				if self.uses_routed_identifiers() {
					tracing::warn!(
						progress_token = %pn.params.progress_token.0,
						"dropping progress notification: failed to decode routed progress token"
					);
					return Ok(accepted_response());
				}
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(ClientNotification::ProgressNotification(pn.clone()), &ctx)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "progress notification fanout failed; ignoring");
					}
				}
			},
			ClientNotification::CustomNotification(cn) => {
				let fallback_notification = match RoutedCustomNotification::parse(cn) {
					RoutedCustomNotification::ElicitationResponse {
						mut original,
						mut params,
					} => {
						let routed_elicitation_id = params.typed.elicitation_id.clone();
						if let Ok((server_name, original_elicitation_id)) =
							self.decode_upstream_elicitation_id(&routed_elicitation_id)
						{
							if !self.take_active_elicitation(
								&routed_elicitation_id,
								&server_name,
								&original_elicitation_id,
							) {
								tracing::warn!(
									%server_name,
									elicitation_id = %original_elicitation_id,
									"dropping elicitation response: no active gateway-issued elicitation"
								);
								return Ok(accepted_response());
							}
							params.typed.elicitation_id = original_elicitation_id;
							let params_value = match serde_json::to_value(&params) {
								Ok(v) => v,
								Err(e) => {
									tracing::warn!(
										%server_name,
										elicitation_id = %params.typed.elicitation_id,
										error = %e,
										"dropping elicitation response: failed to serialize params"
									);
									return Ok(accepted_response());
								},
							};
							original.params = Some(params_value);
							match self.upstreams.get(&server_name) {
								Ok(us) => {
									if let Err(e) = us
										.generic_notification(ClientNotification::CustomNotification(original), &ctx)
										.await
									{
										if !self.allow_degraded {
											return Err(e);
										}
										tracing::warn!(%server_name, ?e, "targeted elicitation response failed");
									}
								},
								Err(_) => {
									tracing::warn!(
										%server_name,
										elicitation_id = %params.typed.elicitation_id,
										"dropping elicitation response: upstream not found"
									);
								},
							}
							return Ok(accepted_response());
						}
						tracing::warn!(
							elicitation_id = %params.typed.elicitation_id,
							"dropping elicitation response: failed to decode upstream id"
						);
						return Ok(accepted_response());
					},
					RoutedCustomNotification::Other(notification) => notification,
				};

				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con
						.generic_notification(
							ClientNotification::CustomNotification(fallback_notification.clone()),
							&ctx,
						)
						.await
					{
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "custom notification fanout failed; ignoring");
					}
				}
			},
			notification => {
				for (name, con) in self.upstreams.iter_named() {
					if let Err(e) = con.generic_notification(notification.clone(), &ctx).await {
						if !self.allow_degraded {
							return Err(e);
						}
						tracing::warn!(%name, ?e, "upstream notification failed; ignoring");
					}
				}
			},
		}

		Ok(accepted_response())
	}

	pub async fn send_client_message(
		&self,
		service_name: String,
		message: ClientJsonRpcMessage,
		ctx: IncomingRequestContext,
	) -> Result<Response, UpstreamError> {
		let Ok(us) = self.upstreams.get(&service_name) else {
			return Err(UpstreamError::InvalidRequest(format!(
				"unknown service {service_name}"
			)));
		};
		us.send_client_message(message, &ctx).await?;
		Ok(accepted_response())
	}
}

fn messages_to_response(
	id: RequestId,
	stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Send + 'static,
) -> Result<Response, UpstreamError> {
	let stream = stream.map(move |rpc| {
		let r = match rpc {
			Ok(rpc) => rpc,
			Err(e) => {
				ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), id.clone())
			},
		};
		ServerSseMessage {
			event_id: None,
			message: Arc::new(r),
		}
	});
	Ok(crate::mcp::session::sse_stream_response(stream, None))
}

async fn messages_to_buffered_response(
	id: RequestId,
	mut stream: impl Stream<Item = Result<ServerJsonRpcMessage, ClientError>> + Unpin,
) -> Result<Response, UpstreamError> {
	let mut messages = Vec::new();
	while let Some(rpc) = stream.next().await {
		let rpc = match rpc {
			Ok(rpc) => rpc,
			Err(e) => {
				ServerJsonRpcMessage::error(ErrorData::internal_error(e.to_string(), None), id.clone())
			},
		};
		messages.push(rpc);
	}

	let Some(first) = messages.first() else {
		return Err(UpstreamError::InvalidRequest(
			"all upstream streams ended before terminal response".to_string(),
		));
	};

	if messages.len() == 1
		&& matches!(
			first,
			ServerJsonRpcMessage::Response(_) | ServerJsonRpcMessage::Error(_)
		) {
		let body = serde_json::to_vec(first).map_err(|e| {
			UpstreamError::InvalidRequest(format!("failed to serialize jsonrpc response: {e}"))
		})?;
		return ::http::Response::builder()
			.status(StatusCode::OK)
			.header(
				http::header::CONTENT_TYPE,
				rmcp::transport::common::http_header::JSON_MIME_TYPE,
			)
			.body(crate::http::Body::from(body))
			.map_err(|e| UpstreamError::InvalidRequest(format!("failed to build json response: {e}")));
	}

	let stream = futures_util::stream::iter(messages.into_iter().map(|message| ServerSseMessage {
		event_id: None,
		message: Arc::new(message),
	}));
	Ok(crate::mcp::session::sse_stream_response(stream, None))
}

fn accepted_response() -> Response {
	::http::Response::builder()
		.status(StatusCode::ACCEPTED)
		.body(crate::http::Body::empty())
		.expect("valid response")
}
