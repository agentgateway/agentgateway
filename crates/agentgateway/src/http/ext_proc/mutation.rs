use std::convert::Infallible;

use bytes::Bytes;
use http_body::Frame;
use tokio::sync::mpsc::Sender;
use tracing::{trace, warn};

use super::headers::{apply_header_mutations_request, apply_header_mutations_response};
use super::proto::body_mutation::Mutation;
use super::proto::processing_response::Response;
use super::{Error, ExtProcDynamicMetadata};
use crate::http::ext_proc::proto::{
	BodyMutation, BodyResponse, CommonResponse, HeadersResponse, ImmediateResponse,
	ProcessingResponse,
};
use crate::http::{self, PolicyResponse, envoy_proto_common};
use crate::proxy::ProxyError;

pub(super) fn to_immediate_response(rp: &ProcessingResponse) -> Option<PolicyResponse> {
	match &rp.response {
		Some(Response::ImmediateResponse(ir)) => {
			let ImmediateResponse {
				status,
				headers,
				body,
				grpc_status: _,
				details: _,
			} = ir;
			let rb =
				::http::response::Builder::new().status(status.map(|s| s.code).unwrap_or(200) as u16);

			let mut resp = rb
				.body(http::Body::from(body.to_string()))
				.map_err(|e| ProxyError::Processing(e.into()))
				.unwrap();
			apply_header_mutations_response(&mut resp, headers.as_ref());
			Some(crate::http::PolicyResponse {
				direct_response: Some(resp),
				response_headers: None,
			})
		},
		_ => None,
	}
}

pub(super) fn request_body_response_has_no_mutation(presp: &ProcessingResponse) -> bool {
	matches!(
		&presp.response,
		Some(Response::RequestBody(BodyResponse { response: None }))
	) || matches!(
		&presp.response,
		Some(Response::RequestBody(BodyResponse { response: Some(cr) })) if cr.body_mutation.is_none()
	)
}

pub(super) fn response_body_response_has_no_mutation(presp: &ProcessingResponse) -> bool {
	matches!(
		&presp.response,
		Some(Response::ResponseBody(BodyResponse { response: None }))
	) || matches!(
		&presp.response,
		Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) if cr.body_mutation.is_none()
	)
}

fn common_response_has_streamed_body_mutation(cr: &CommonResponse) -> bool {
	matches!(
		&cr.body_mutation,
		Some(BodyMutation {
			mutation: Some(Mutation::StreamedResponse(_))
		})
	)
}

pub(super) fn request_response_has_streamed_body_mutation(presp: &ProcessingResponse) -> bool {
	match &presp.response {
		Some(Response::RequestHeaders(HeadersResponse { response: Some(cr) }))
		| Some(Response::RequestBody(BodyResponse { response: Some(cr) })) => {
			common_response_has_streamed_body_mutation(cr)
		},
		_ => false,
	}
}

pub(super) fn response_response_has_streamed_body_mutation(presp: &ProcessingResponse) -> bool {
	match &presp.response {
		Some(Response::ResponseHeaders(HeadersResponse { response: Some(cr) }))
		| Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) => {
			common_response_has_streamed_body_mutation(cr)
		},
		_ => false,
	}
}

// handle_response_for_request_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
pub(super) async fn handle_response_for_request_mutation(
	had_body: bool,
	allow_header_mutations: bool,
	validate_content_length: bool,
	mut req: Option<&mut http::Request>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> Result<(bool, bool), Error> {
	if let Some(dm) = &presp.dynamic_metadata {
		if let Some(req) = req.as_mut() {
			if let Err(e) = extract_dynamic_metadata(req, dm) {
				warn!("Failed to extract ext_proc dynamic metadata: {}", e);
			}
		} else if !dm.fields.is_empty() {
			warn!(
				"ext_proc server sent dynamic_metadata after headers were processed; \
					 metadata cannot be attached and will be ignored. Consider sending \
					 metadata in the RequestHeaders response instead."
			);
		}
	}

	let res = matches!(&presp.response, Some(Response::RequestHeaders(_)));
	let is_body_response = matches!(&presp.response, Some(Response::RequestBody(_)));
	let cr = match presp.response {
		Some(Response::RequestHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return Ok((true, !had_body));
		},
		Some(Response::RequestHeaders(HeadersResponse { response: Some(cr) })) => {
			trace!("got request headers back");
			cr
		},
		Some(Response::RequestBody(BodyResponse { response: None })) => {
			trace!("got empty request body back");
			return Ok((false, true));
		},
		Some(Response::RequestBody(BodyResponse { response: Some(cr) })) => {
			trace!("got request body back");
			cr
		},
		Some(Response::RequestTrailers(_)) => {
			trace!("got request trailers back");
			return Ok((false, true));
		},
		Some(Response::ImmediateResponse(_)) => {
			if req.is_none() {
				trace!("immediate response received after request sent; will apply only on the response");
			}
			// Handled out of this function.
			return Ok((true, true));
		},
		msg => {
			warn!("ignoring response during request {msg:?}");
			return Ok((false, false));
		},
	};
	if allow_header_mutations && let Some(req) = req.as_deref_mut() {
		apply_header_mutations_request(req, cr.header_mutation.as_ref());
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		if !had_body {
			trace!("ignoring request body mutation when no body is expected");
			return Ok((res, true));
		}
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let _ = body_tx.send(Ok(Frame::data(bb.body))).await;
				trace!(eos, "got stream request body");
				return Ok((res, eos));
			},
			Mutation::Body(b) => {
				// Used in Buffered mode: ext_proc replaces the entire body at once.
				if validate_content_length && let Some(req) = req.as_deref() {
					validate_content_length_header(req.headers(), b.len(), "request")?;
				}
				let _ = body_tx.send(Ok(Frame::data(b))).await;
				return Ok((true, true));
			},
			Mutation::ClearBody(_) => {
				// Body cleared: signal end-of-stream with no data.
				if validate_content_length && let Some(req) = req.as_deref() {
					validate_content_length_header(req.headers(), 0, "request")?;
				}
				return Ok((true, true));
			},
		}
	} else if !had_body {
		trace!("got headers back and do not expect body; we are done");
		return Ok((res, true));
	} else if is_body_response {
		trace!("got request body back without body mutation; forwarding original body");
		return Ok((true, true));
	}
	trace!("still waiting for response...");
	Ok((res, false))
}

fn merge_dynamic_metadata(
	extensions: &mut ::http::Extensions,
	metadata: &prost_wkt_types::Struct,
) -> Result<(), Error> {
	let mut dynamic_metadata = extensions
		.remove::<ExtProcDynamicMetadata>()
		.unwrap_or_default();

	for (key, value) in &metadata.fields {
		let json_val = envoy_proto_common::prost_value_to_json(value)
			.map_err(|e| Error::MetadataConversion(format!("failed to convert key '{}': {}", key, e)))?;
		dynamic_metadata.0.insert(key.clone(), json_val);
	}

	if !dynamic_metadata.0.is_empty() {
		extensions.insert(dynamic_metadata);
	}

	Ok(())
}

// handle_response_for_response_mutation handles a single ext_proc response. If it returns 'true' we are done processing.
pub(super) async fn handle_response_for_response_mutation(
	had_body: bool,
	allow_header_mutations: bool,
	validate_content_length: bool,
	mut resp: Option<&mut http::Response>,
	body_tx: &mut Sender<Result<Frame<Bytes>, Infallible>>,
	presp: ProcessingResponse,
) -> Result<(bool, bool), Error> {
	if let Some(dm) = &presp.dynamic_metadata {
		if let Some(resp) = resp.as_mut() {
			if let Err(e) = extract_dynamic_metadata_response(resp, dm) {
				warn!("Failed to extract ext_proc dynamic metadata: {}", e);
			}
		} else if !dm.fields.is_empty() {
			warn!(
				"ext_proc server sent dynamic_metadata after response headers were processed; \
				 metadata cannot be attached and will be ignored. Consider sending \
				 metadata in the ResponseHeaders response instead."
			);
		}
	}

	let res = matches!(&presp.response, Some(Response::ResponseHeaders(_)));
	let is_body_response = matches!(&presp.response, Some(Response::ResponseBody(_)));
	let cr = match presp.response {
		Some(Response::ResponseHeaders(HeadersResponse { response: None })) => {
			trace!("no headers");
			return Ok((res, false));
		},
		Some(Response::ResponseHeaders(HeadersResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: Some(cr) })) => cr,
		Some(Response::ResponseBody(BodyResponse { response: None })) => {
			trace!("got empty response body back");
			return Ok((res, true));
		},
		Some(Response::ResponseTrailers(_)) => {
			trace!("got response trailers back");
			return Ok((res, true));
		},
		Some(Response::ImmediateResponse(_)) => {
			warn!("received ImmediateResponse during response-body continuation; treating as terminal");
			return Ok((true, true));
		},
		msg => {
			warn!("ignoring {msg:?}");
			return Ok((res, false));
		},
	};
	if allow_header_mutations && let Some(resp) = resp.as_deref_mut() {
		apply_header_mutations_response(resp, cr.header_mutation.as_ref());
	}
	if let Some(BodyMutation { mutation: Some(b) }) = cr.body_mutation {
		if !had_body {
			trace!("ignoring response body mutation when no body is expected");
			return Ok((res, true));
		}
		match b {
			Mutation::StreamedResponse(bb) => {
				let eos = bb.end_of_stream;
				let _ = body_tx.send(Ok(Frame::data(bb.body))).await;
				trace!(%eos, "got body chunk");
				return Ok((res, eos));
			},
			Mutation::Body(b) => {
				if validate_content_length && let Some(resp) = resp.as_deref() {
					validate_content_length_header(resp.headers(), b.len(), "response")?;
				}
				let _ = body_tx.send(Ok(Frame::data(b))).await;
				return Ok((true, true));
			},
			Mutation::ClearBody(_) => {
				if validate_content_length && let Some(resp) = resp.as_deref() {
					validate_content_length_header(resp.headers(), 0, "response")?;
				}
				return Ok((true, true));
			},
		}
	} else if !had_body {
		trace!("got headers back and do not expect body; we are done");
		return Ok((res, true));
	} else if is_body_response {
		trace!("got response body back without body mutation; forwarding original body");
		return Ok((true, true));
	}
	trace!("still waiting for response for response...");
	Ok((res, false))
}

fn validate_content_length_header(
	headers: &::http::HeaderMap,
	mutated_body_len: usize,
	direction: &'static str,
) -> Result<(), Error> {
	let Some(content_length) = headers.get(::http::header::CONTENT_LENGTH) else {
		return Ok(());
	};
	let actual_len = content_length
		.to_str()
		.ok()
		.and_then(|v| v.parse::<usize>().ok());
	if actual_len == Some(mutated_body_len) {
		return Ok(());
	}
	let actual = content_length.to_str().unwrap_or("<non-utf8>");
	Err(Error::BodyMutation(format!(
		"{direction} body mutation content-length mismatch: header value {actual:?} does not match mutated body length {mutated_body_len}"
	)))
}

pub(crate) fn extract_dynamic_metadata(
	req: &mut http::Request,
	metadata: &prost_wkt_types::Struct,
) -> Result<(), Error> {
	merge_dynamic_metadata(req.extensions_mut(), metadata)
}

fn extract_dynamic_metadata_response(
	resp: &mut http::Response,
	metadata: &prost_wkt_types::Struct,
) -> Result<(), Error> {
	merge_dynamic_metadata(resp.extensions_mut(), metadata)
}
