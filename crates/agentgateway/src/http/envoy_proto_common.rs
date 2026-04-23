use ::http::HeaderMap;
use prost_wkt_types::{Struct, Value as ProstValue};
use serde_json::Value as JsonValue;
use tracing::warn;

use crate::http::{
	HeaderMutationAction, HeaderName, HeaderOrPseudo, HeaderOrPseudoValue, HeaderValue,
	RequestOrResponse,
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

pub fn resolve_header_mutation_action(header: &ProtoHeaderValueOption) -> HeaderMutationAction {
	match resolve_append_action(header) {
		HeaderAppendAction::AppendIfExistsOrAdd => HeaderMutationAction::AppendIfExistsOrAdd,
		HeaderAppendAction::AddIfAbsent => HeaderMutationAction::AddIfAbsent,
		HeaderAppendAction::OverwriteIfExistsOrAdd => {
			HeaderMutationAction::OverwriteIfExistsOrAdd
		},
		HeaderAppendAction::OverwriteIfExists => HeaderMutationAction::OverwriteIfExists,
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

pub fn apply_header_option(
	rr: &mut RequestOrResponse<'_>,
	header: &ProtoHeaderValueOption,
)  {
	let Some(ref h) = header.header else {
		warn!("Invalid header mutation: no header provided");
		return;
	};
	let Ok(key) = HeaderOrPseudo::try_from(h.key.as_str()) else {
		warn!("Invalid header mutation: {} is not a valid header", h.key);
		return;
	};
	let Some(raw) = raw_or_value_bytes(h) else {
		warn!("Invalid header mutation: value is not valid",);
		return;
	};
	let Some(value) = HeaderOrPseudoValue::from_raw(&key, raw) else {
		warn!("Invalid header mutation: value is not valid",);
		return;
	};
	rr.apply_header(&key, Some(value), resolve_header_mutation_action(header));
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::{Body, HeaderMutationAction, HeaderOrPseudo, RequestOrResponse};
	use ::http::{HeaderMap, HeaderName, header};

	fn header_option(
		key: &str,
		value: &str,
		raw_value: &[u8],
		append: Option<bool>,
		append_action: i32,
	) -> ProtoHeaderValueOption {
		ProtoHeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: key.to_string(),
				value: value.to_string(),
				raw_value: raw_value.to_vec(),
			}),
			append,
			append_action,
		}
	}

	fn apply_header_map_options(headers: &mut HeaderMap, options: &[ProtoHeaderValueOption]) {
		for option in options {
			let header = option.header.as_ref().unwrap();
			let name = HeaderName::from_bytes(header.key.as_bytes()).unwrap();
			assert!(apply_header_value_option(headers, &name, option));
		}
	}

	fn apply_request_options(
		req: &mut ::http::Request<Body>,
		options: &[ProtoHeaderValueOption],
	) {
		let mut rr = RequestOrResponse::Request(req);
		for option in options {
			apply_header_option(&mut rr, option);
		}
	}

	fn apply_response_options(
		resp: &mut ::http::Response<Body>,
		options: &[ProtoHeaderValueOption],
	) {
		let mut rr = RequestOrResponse::Response(resp);
		for option in options {
			apply_header_option(&mut rr, option);
		}
	}

	#[test]
	fn test_default_append_action_overwrite() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "old".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[header_option("existing", "", b"new", None, 0)],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "new");
	}

	#[test]
	fn test_append_if_exists_or_add() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "value1".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option(
					"existing",
					"",
					b"value2",
					Some(true),
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
				header_option(
					"new",
					"",
					b"added",
					Some(true),
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "value1");
		assert_eq!(values[1], "value2");
		assert_eq!(headers.get("new").unwrap(), "added");
	}

	#[test]
	fn test_add_if_absent() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "value1".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option(
					"existing",
					"",
					b"should-not-add",
					None,
					HeaderAppendAction::AddIfAbsent as i32,
				),
				header_option(
					"new",
					"",
					b"added",
					None,
					HeaderAppendAction::AddIfAbsent as i32,
				),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "value1");
		assert_eq!(headers.get("new").unwrap(), "added");
	}

	#[test]
	fn test_overwrite_if_exists_or_add() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "old-value".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option(
					"existing",
					"",
					b"overwritten",
					None,
					HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
				),
				header_option(
					"new",
					"",
					b"added",
					None,
					HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
				),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "overwritten");
		assert_eq!(headers.get("new").unwrap(), "added");
	}

	#[test]
	fn test_overwrite_if_exists() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "old-value".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option(
					"existing",
					"",
					b"overwritten",
					None,
					HeaderAppendAction::OverwriteIfExists as i32,
				),
				header_option(
					"new",
					"",
					b"should-not-add",
					None,
					HeaderAppendAction::OverwriteIfExists as i32,
				),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "overwritten");
		assert!(headers.get("new").is_none());
	}

	#[test]
	fn test_apply_header_option_request() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com")
			.header("existing", "value1")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[header_option(
				"existing",
				"",
				b"value2",
				Some(true),
				HeaderAppendAction::AppendIfExistsOrAdd as i32,
			)],
		);

		let values: Vec<_> = req.headers().get_all("existing").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "value1");
		assert_eq!(values[1], "value2");
	}

	#[test]
	fn test_apply_pseudo_headers_request_with_raw_value() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/old-path")
			.method("GET")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[
				header_option(":method", "", b"POST", None, 0),
				header_option(":path", "", b"/new-path", None, 0),
				header_option(":authority", "", b"new-host.com", None, 0),
				header_option(":scheme", "", b"https", None, 0),
			],
		);

		assert_eq!(req.method(), "POST");
		assert_eq!(req.uri().path(), "/new-path");
		assert_eq!(req.uri().scheme_str(), Some("https"));
		assert_eq!(req.uri().authority().unwrap().as_str(), "new-host.com");
	}

	#[test]
	fn test_apply_pseudo_headers_request_with_value_field() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/old-path")
			.method("GET")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[
				header_option(":method", "PUT", b"", None, 0),
				header_option(":path", "/updated-path", b"", None, 0),
			],
		);

		assert_eq!(req.method(), "PUT");
		assert_eq!(req.uri().path(), "/updated-path");
	}

	#[test]
	fn test_pseudo_headers_request_raw_value_precedence() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/path")
			.method("GET")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[header_option(":method", "PUT", b"DELETE", None, 0)],
		);

		assert_eq!(req.method(), "DELETE");
	}

	#[test]
	fn test_apply_header_option_response() {
		let mut resp = ::http::Response::builder()
			.status(200)
			.header("existing", "value1")
			.body(Body::empty())
			.unwrap();

		apply_response_options(
			&mut resp,
			&[header_option(
				"existing",
				"",
				b"value2",
				Some(true),
				HeaderAppendAction::AppendIfExistsOrAdd as i32,
			)],
		);

		let values: Vec<_> = resp.headers().get_all("existing").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "value1");
		assert_eq!(values[1], "value2");
	}

	#[test]
	fn test_apply_pseudo_headers_response_with_raw_value() {
		let mut resp = ::http::Response::builder()
			.status(200)
			.header("x-test", "value")
			.body(Body::empty())
			.unwrap();

		apply_response_options(
			&mut resp,
			&[header_option(":status", "", b"404", None, 0)],
		);

		assert_eq!(resp.status(), 404);
		assert_eq!(resp.headers().get("x-test").unwrap(), "value");
	}

	#[test]
	fn test_apply_pseudo_headers_response_with_value_field() {
		let mut resp = ::http::Response::builder()
			.status(200)
			.body(Body::empty())
			.unwrap();

		apply_response_options(
			&mut resp,
			&[header_option(":status", "201", b"", None, 0)],
		);

		assert_eq!(resp.status(), 201);
	}

	#[test]
	fn test_pseudo_headers_response_raw_value_precedence() {
		let mut resp = ::http::Response::builder()
			.status(200)
			.body(Body::empty())
			.unwrap();

		apply_response_options(
			&mut resp,
			&[header_option(":status", "500", b"403", None, 0)],
		);

		assert_eq!(resp.status(), 403);
	}

	#[test]
	fn test_apply_mixed_headers_and_pseudo_headers_request() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/path")
			.method("GET")
			.header("x-custom", "old-value")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[
				header_option(":method", "", b"POST", None, 0),
				header_option(
					"x-custom",
					"",
					b"new-value",
					None,
					HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
				),
				header_option(
					"x-new-header",
					"added",
					b"",
					None,
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
			],
		);

		assert_eq!(req.method(), "POST");
		assert_eq!(req.headers().get("x-custom").unwrap(), "new-value");
		assert_eq!(req.headers().get("x-new-header").unwrap(), "added");
	}

	#[test]
	fn test_apply_host_header_request_lifts_to_authority() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/path")
			.method("GET")
			.header("host", "stale.example.com")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[header_option(
				"host",
				"",
				b"rewritten.example.com:8443",
				Some(true),
				HeaderAppendAction::AppendIfExistsOrAdd as i32,
			)],
		);

		assert_eq!(
			req.uri().authority().unwrap().as_str(),
			"rewritten.example.com:8443"
		);
		assert!(req.headers().get("host").is_none());
	}

	#[test]
	fn test_apply_host_header_request_add_if_absent_sets_authority() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/path")
			.method("GET")
			.body(Body::empty())
			.unwrap();

		apply_request_options(
			&mut req,
			&[header_option(
				"host",
				"",
				b"ignored.example.com",
				None,
				HeaderAppendAction::AddIfAbsent as i32,
			)],
		);

		assert_eq!(req.uri().authority().unwrap().as_str(), "ignored.example.com");
		assert!(req.headers().get("host").is_none());
	}

	#[test]
	fn test_apply_host_header_request_remove_keeps_authority() {
		let mut req = ::http::Request::builder()
			.uri("http://example.com/path")
			.method("GET")
			.header("host", "stale.example.com")
			.body(Body::empty())
			.unwrap();

		let mut rr = RequestOrResponse::Request(&mut req);
		rr.apply_header(
			&HeaderOrPseudo::Header(header::HOST),
			None,
			HeaderMutationAction::OverwriteIfExistsOrAdd,
		);

		assert_eq!(req.uri().authority().unwrap().as_str(), "example.com");
		assert!(req.headers().get("host").is_none());
	}

	#[test]
	fn test_apply_mixed_headers_and_pseudo_headers_response() {
		let mut resp = ::http::Response::builder()
			.status(200)
			.header("x-custom", "old-value")
			.body(Body::empty())
			.unwrap();

		apply_response_options(
			&mut resp,
			&[
				header_option(":status", "", b"201", None, 0),
				header_option(
					"x-custom",
					"",
					b"new-value",
					None,
					HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
				),
				header_option(
					"x-new-header",
					"added",
					b"",
					None,
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
			],
		);

		assert_eq!(resp.status(), 201);
		assert_eq!(resp.headers().get("x-custom").unwrap(), "new-value");
		assert_eq!(resp.headers().get("x-new-header").unwrap(), "added");
	}

	#[test]
	fn test_deprecated_append_true() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "value1".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option("existing", "", b"value2", Some(true), 0),
				header_option("new", "", b"added", Some(true), 0),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "value1");
		assert_eq!(values[1], "value2");
		assert_eq!(headers.get("new").unwrap(), "added");
	}

	#[test]
	fn test_deprecated_append_false() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "old-value".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[header_option("existing", "", b"overwritten", Some(false), 0)],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "overwritten");
	}

	#[test]
	fn test_value_field_instead_of_raw_value() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "value1".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[
				header_option(
					"existing",
					"value2",
					b"",
					Some(true),
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
				header_option(
					"new",
					"added",
					b"",
					Some(true),
					HeaderAppendAction::AppendIfExistsOrAdd as i32,
				),
			],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 2);
		assert_eq!(values[0], "value1");
		assert_eq!(values[1], "value2");
		assert_eq!(headers.get("new").unwrap(), "added");
	}

	#[test]
	fn test_raw_value_takes_precedence_over_value() {
		let mut headers = HeaderMap::new();

		apply_header_map_options(
			&mut headers,
			&[header_option(
				"test",
				"should-not-use",
				b"raw-value-wins",
				None,
				HeaderAppendAction::AppendIfExistsOrAdd as i32,
			)],
		);

		assert_eq!(headers.get("test").unwrap(), "raw-value-wins");
	}

	#[test]
	fn test_append_action_priority_over_deprecated_append() {
		let mut headers = HeaderMap::new();
		headers.insert("existing", "value1".parse().unwrap());

		apply_header_map_options(
			&mut headers,
			&[header_option(
				"existing",
				"",
				b"overwritten",
				Some(true),
				HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
			)],
		);

		let values: Vec<_> = headers.get_all("existing").iter().collect();
		assert_eq!(values.len(), 1);
		assert_eq!(values[0], "overwritten");
	}
}
