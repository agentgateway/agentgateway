use ::http::HeaderMap;
use prost_wkt_types::{Struct, Value as ProstValue};
use serde_json::Value as JsonValue;
use tracing::warn;

use crate::http::{
	HeaderName, HeaderOrPseudo, HeaderValue, RequestOrResponse, apply_header_or_pseudo,
};
use crate::proxy::ProxyError;

type ProtoHeaderValue = protos::envoy::service::common::v3::HeaderValue;
type ProtoHeaderValueOption = protos::envoy::service::common::v3::HeaderValueOption;
type HeaderAppendAction =
	protos::envoy::service::common::v3::header_value_option::HeaderAppendAction;

pub fn raw_or_value_bytes(header: &ProtoHeaderValue) -> Option<&[u8]> {
	if !header.raw_value.is_empty() {
		Some(header.raw_value.as_slice())
	} else if !header.value.is_empty() {
		Some(header.value.as_bytes())
	} else {
		None
	}
}

pub fn decode_header_value(
	header: &ProtoHeaderValue,
) -> Result<Option<HeaderValue>, http::header::InvalidHeaderValue> {
	let Some(raw) = raw_or_value_bytes(header) else {
		return Ok(None);
	};
	HeaderValue::from_bytes(raw).map(Some)
}

pub fn resolve_append_action(header: &ProtoHeaderValueOption) -> HeaderAppendAction {
	if header.append_action == 0 {
		match header.append {
			Some(true) => HeaderAppendAction::AppendIfExistsOrAdd,
			_ => HeaderAppendAction::OverwriteIfExistsOrAdd,
		}
	} else {
		match HeaderAppendAction::try_from(header.append_action) {
			Ok(action) => action,
			Err(_) => {
				warn!(
					"Unexpected header append_action `{:?}` falling back to APPEND_IF_EXISTS_OR_ADD",
					header.append_action
				);
				HeaderAppendAction::AppendIfExistsOrAdd
			},
		}
	}
}

pub fn apply_header_value_option(
	headers: &mut HeaderMap,
	name: &HeaderName,
	header: &ProtoHeaderValueOption,
) -> bool {
	let Some(ref h) = header.header else {
		return false;
	};
	let Ok(value) = decode_header_value(h) else {
		warn!("Invalid header value for key: {}", h.key);
		return false;
	};
	let Some(value) = value else {
		return false;
	};
	match resolve_append_action(header) {
		HeaderAppendAction::AppendIfExistsOrAdd => {
			headers.append(name, value);
		},
		HeaderAppendAction::AddIfAbsent => {
			if !headers.contains_key(name) {
				headers.insert(name, value);
			}
		},
		HeaderAppendAction::OverwriteIfExistsOrAdd => {
			headers.insert(name, value);
		},
		HeaderAppendAction::OverwriteIfExists => {
			if headers.contains_key(name) {
				headers.insert(name, value);
			}
		},
	}
	true
}

pub fn apply_pseudo_header_option(
	rr: &mut RequestOrResponse<'_>,
	header: &ProtoHeaderValueOption,
) -> bool {
	let Some(ref h) = header.header else {
		return false;
	};
	let Ok(pseudo) = HeaderOrPseudo::try_from(h.key.as_str()) else {
		return false;
	};
	let Some(raw) = raw_or_value_bytes(h) else {
		return false;
	};
	apply_header_or_pseudo(rr, &pseudo, raw)
}

pub fn apply_header_value(headers: &mut HeaderMap, header: &ProtoHeaderValue) -> bool {
	let Ok(name) = HeaderName::from_bytes(header.key.as_bytes()) else {
		return false;
	};
	let Ok(value) = decode_header_value(header) else {
		return false;
	};
	let Some(value) = value else {
		return false;
	};
	headers.insert(name, value);
	true
}

pub fn prost_value_to_json(value: &ProstValue) -> Result<JsonValue, ProxyError> {
	serde_json::to_value(value).map_err(|e| ProxyError::Processing(e.into()))
}

pub fn json_to_struct(value: JsonValue) -> Result<Struct, ProxyError> {
	serde_json::from_value(value).map_err(|e| ProxyError::Processing(e.into()))
}

pub fn json_to_prost_value(value: JsonValue) -> Result<ProstValue, ProxyError> {
	serde_json::from_value(value).map_err(|e| ProxyError::Processing(e.into()))
}
