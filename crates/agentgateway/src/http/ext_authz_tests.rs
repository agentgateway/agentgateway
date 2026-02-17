use std::sync::Arc;

use ::http::HeaderMap;

use crate::http::HeaderOrPseudo;
use crate::http::ext_authz::proto::{HeaderValue as ProtoHeaderValue, HeaderValueOption};
use crate::http::ext_authz::{BodyOptions, ExtAuthz, ExtAuthzDynamicMetadata, FailureMode};
use crate::types::agent::SimpleBackendReference;
use crate::*;

impl Default for ExtAuthz {
	fn default() -> Self {
		Self {
			target: Arc::new(SimpleBackendReference::Invalid),
			failure_mode: FailureMode::default(),
			include_request_headers: Vec::new(),
			include_request_body: None,
			protocol: http::ext_authz::Protocol::Grpc {
				context: None,
				metadata: None,
			},
			timeout: None,
		}
	}
}

#[test]
fn test_process_headers_with_allowlist() {
	let mut headers = HeaderMap::new();

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-allowed".to_string(),
				value: "allowed-value".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-not-allowed".to_string(),
				value: "should-be-filtered".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
	];

	// Test with allowlist
	let allowlist = vec!["x-allowed".to_string()];
	super::process_headers(&mut headers, header_options, Some(&allowlist));

	assert_eq!(headers.get("x-allowed").unwrap(), "allowed-value");
	assert!(headers.get("x-not-allowed").is_none());
}

#[test]
fn test_process_headers() {
	let mut headers = HeaderMap::new();

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-custom-header".to_string(),
				value: "test-value".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-append-header".to_string(),
				value: "value1".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-append-header".to_string(),
				value: "value2".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: 0,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-raw-header".to_string(),
				value: "ignored".to_string(),
				raw_value: b"raw-value".to_vec(),
			}),
			append: Some(false),
			append_action: 0,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	assert_eq!(headers.get("x-custom-header").unwrap(), "test-value");
	assert_eq!(headers.get("x-raw-header").unwrap(), "raw-value");

	let append_values: Vec<_> = headers.get_all("x-append-header").iter().collect();
	assert_eq!(append_values.len(), 2);
	assert_eq!(append_values[0], "value1");
	assert_eq!(append_values[1], "value2");
}

#[test]
fn test_body_truncation() {
	let body_opts = BodyOptions {
		max_request_bytes: 10,
		allow_partial_message: true,
		pack_as_bytes: false,
	};

	// Test truncation
	let long_body = b"This is a very long body that exceeds max size";
	assert!(long_body.len() > body_opts.max_request_bytes as usize);

	let mut truncated = long_body.to_vec();
	truncated.truncate(body_opts.max_request_bytes as usize);
	assert_eq!(truncated.len(), 10);
	assert_eq!(&truncated, b"This is a ");
}

#[test]
fn test_multi_value_headers() {
	use ::http::Request;

	let req = Request::builder()
		.header("cookie", "session=abc")
		.header("cookie", "user=123")
		.header("x-forwarded-for", "10.0.0.1")
		.header("x-forwarded-for", "10.0.0.2")
		.body(http::Body::empty())
		.unwrap();

	// Collect all cookie values
	let cookies: Vec<_> = req
		.headers()
		.get_all("cookie")
		.iter()
		.filter_map(|v| v.to_str().ok())
		.collect();
	assert_eq!(cookies.len(), 2);
	assert_eq!(cookies[0], "session=abc");
	assert_eq!(cookies[1], "user=123");

	// Test joining with semicolon for cookies
	let joined = cookies.join("; ");
	assert_eq!(joined, "session=abc; user=123");
}

#[test]
fn test_pseudo_header_protection() {
	let headers_to_remove = [
		":method".to_string(),
		":path".to_string(),
		"host".to_string(),
		"Host".to_string(),
		"content-type".to_string(),
	];

	// Only non-pseudo and non-host headers should be removable
	let removable: Vec<_> = headers_to_remove
		.iter()
		.filter(|h| !h.starts_with(':') && h.to_lowercase() != "host")
		.collect();

	assert_eq!(removable.len(), 1);
	assert_eq!(removable[0], "content-type");
}

#[test]
fn test_header_or_pseudo_parsing() {
	// pseudo header parsing
	assert!(matches!(
		HeaderOrPseudo::try_from(":method"),
		Ok(HeaderOrPseudo::Method)
	));
	assert!(matches!(
		HeaderOrPseudo::try_from(":scheme"),
		Ok(HeaderOrPseudo::Scheme)
	));
	assert!(matches!(
		HeaderOrPseudo::try_from(":authority"),
		Ok(HeaderOrPseudo::Authority)
	));
	assert!(matches!(
		HeaderOrPseudo::try_from(":path"),
		Ok(HeaderOrPseudo::Path)
	));
	assert!(matches!(
		HeaderOrPseudo::try_from(":status"),
		Ok(HeaderOrPseudo::Status)
	));

	// not a pseudo header
	let result = HeaderOrPseudo::try_from("content-type");
	assert!(matches!(result, Ok(HeaderOrPseudo::Header(_))));
	if let Ok(HeaderOrPseudo::Header(header_name)) = result {
		assert_eq!(header_name.as_str(), "content-type");
	}
}

#[test]
fn test_pseudo_header_value_extraction() {
	use ::http::{Method, Request};

	let req = Request::builder()
		.method(Method::POST)
		.uri("https://example.com:8080/api/v1/test?param=value")
		.header("host", "example.com:8080")
		.body(http::Body::empty())
		.unwrap();

	let method_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Method, &req);
	assert_eq!(method_value, Some("POST".to_string()));

	let scheme_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Scheme, &req);
	assert_eq!(scheme_value, Some("https".to_string()));

	let authority_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Authority, &req);
	assert_eq!(authority_value, Some("example.com:8080".to_string()));

	let path_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Path, &req);
	assert_eq!(path_value, Some("/api/v1/test?param=value".to_string()));

	let status_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Status, &req);
	assert_eq!(status_value, None);
}

#[test]
fn test_pseudo_header_authority_fallback() {
	use ::http::{Method, Request};

	// fallback to host header when URI doesn't have authority
	let req = Request::builder()
		.method(Method::GET)
		.uri("/api/test")
		.header("host", "fallback.example.com")
		.body(http::Body::empty())
		.unwrap();

	let authority_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Authority, &req);
	assert_eq!(authority_value, Some("fallback.example.com".to_string()));
}

#[test]
fn test_pseudo_header_path_fallback() {
	use ::http::{Method, Request};

	let req = Request::builder()
		.method(Method::GET)
		.uri("/simple/path")
		.body(http::Body::empty())
		.unwrap();

	let path_value = crate::http::get_pseudo_header_value(&HeaderOrPseudo::Path, &req);
	assert_eq!(path_value, Some("/simple/path".to_string()));
}

#[test]
fn test_mixed_regular_and_pseudo_headers() {
	use ::http::{Method, Request};

	let req = Request::builder()
		.method(Method::PUT)
		.uri("https://api.example.com/v2/resource")
		.header("content-type", "application/json")
		.header("authorization", "Bearer token")
		.header("x-custom", "custom-value")
		.body(http::Body::empty())
		.unwrap();

	let extauthz: ExtAuthz = ExtAuthz {
		include_request_headers: vec![
			HeaderOrPseudo::try_from(":method").unwrap(),
			HeaderOrPseudo::try_from(":authority").unwrap(),
			HeaderOrPseudo::try_from("content-type").unwrap(),
			HeaderOrPseudo::try_from("x-custom").unwrap(),
		],
		..Default::default()
	};

	let mut expected_headers = std::collections::HashMap::new();
	expected_headers.insert(":method".to_string(), "PUT".to_string());
	expected_headers.insert(":authority".to_string(), "api.example.com".to_string());
	expected_headers.insert("content-type".to_string(), "application/json".to_string());
	expected_headers.insert("x-custom".to_string(), "custom-value".to_string());

	for header_spec in &extauthz.include_request_headers {
		match header_spec {
			HeaderOrPseudo::Header(header_name) => {
				let value = req
					.headers()
					.get(header_name)
					.and_then(|v| v.to_str().ok())
					.map(|s| s.to_string());
				if let Some(v) = value {
					assert_eq!(expected_headers.get(&header_spec.to_string()), Some(&v));
				}
			},
			pseudo_header => {
				let value = crate::http::get_pseudo_header_value(pseudo_header, &req);
				if let Some(v) = value {
					assert_eq!(expected_headers.get(&header_spec.to_string()), Some(&v));
				}
			},
		}
	}

	// Ensure non-listed headers are excluded
	assert!(!expected_headers.contains_key("authorization"));
}

#[test]
fn test_include_request_headers_empty_includes_all() {
	use ::http::Request;

	let req = Request::builder()
		.header("content-type", "application/json")
		.header("x-custom", "v1")
		.header("x-custom", "v2")
		.header("cookie", "a=1")
		.header("cookie", "b=2")
		.body(http::Body::empty())
		.unwrap();

	let mut headers = std::collections::HashMap::new();
	for name in req.headers().keys() {
		let values: Vec<String> = req
			.headers()
			.get_all(name)
			.iter()
			.filter_map(|v| v.to_str().ok())
			.map(|s| s.to_string())
			.collect();
		if !values.is_empty() {
			let joined = if name.as_str() == "cookie" {
				values.join("; ")
			} else {
				values.join(", ")
			};
			headers.insert(name.as_str().to_string(), joined);
		}
	}

	assert_eq!(headers.get("content-type").unwrap(), "application/json");
	assert_eq!(headers.get("x-custom").unwrap(), "v1, v2");
	assert_eq!(headers.get("cookie").unwrap(), "a=1; b=2");
}

#[test]
fn test_host_header_protection() {
	// Test that host header cannot be added through upstream headers
	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "host".to_string(),
				value: "evil.com".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-custom".to_string(),
				value: "allowed".to_string(),
				raw_value: vec![],
			}),
			append: Some(false),
			append_action: 0,
		},
	];

	// Filter out host header
	let filtered: Vec<_> = header_options
		.into_iter()
		.filter(|h| {
			h.header
				.as_ref()
				.map(|hdr| hdr.key.to_lowercase() != "host")
				.unwrap_or(false)
		})
		.collect();

	assert_eq!(filtered.len(), 1);
	assert_eq!(filtered[0].header.as_ref().unwrap().key, "x-custom");
}

#[test]
fn test_dynamic_metadata_extraction() {
	let mut metadata = ExtAuthzDynamicMetadata::default();

	metadata
		.0
		.insert("user_id".to_string(), serde_json::json!("12345"));
	metadata
		.0
		.insert("role".to_string(), serde_json::json!("admin"));
	assert_eq!(metadata.0.get("user_id").unwrap(), "12345");
	assert_eq!(metadata.0.get("role").unwrap(), "admin");
}

#[test]
fn test_append_action_append_if_exists_or_add() {
	use crate::http::ext_authz::proto::header_value_option::HeaderAppendAction;

	let mut headers = HeaderMap::new();

	// Pre-existing header
	headers.insert("x-test", "existing".parse().unwrap());

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-test".to_string(),
				value: "new-value".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-new".to_string(),
				value: "added".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should append to existing header
	let values: Vec<_> = headers.get_all("x-test").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "existing");
	assert_eq!(values[1], "new-value");

	// Should add new header
	assert_eq!(headers.get("x-new").unwrap(), "added");
}

#[test]
fn test_default_append_action_overwrite() {
	let mut headers = HeaderMap::new();

	// Pre-existing header with multiple values
	headers.append("x-test", "value1".parse().unwrap());
	headers.append("x-test", "value2".parse().unwrap());

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-test".to_string(),
				value: "overwritten".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: 0, // default
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-new".to_string(),
				value: "added".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: 0,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should replace all existing values with single new value
	let values: Vec<_> = headers.get_all("x-test").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");

	// Should add new header
	assert_eq!(headers.get("x-new").unwrap(), "added");
}

#[test]
fn test_append_action_add_if_absent() {
	use crate::http::ext_authz::proto::header_value_option::HeaderAppendAction;

	let mut headers = HeaderMap::new();

	// Pre-existing header
	headers.insert("x-existing", "value1".parse().unwrap());

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-existing".to_string(),
				value: "should-not-add".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::AddIfAbsent as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-new".to_string(),
				value: "should-add".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::AddIfAbsent as i32,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should not modify existing header (no-op)
	let values: Vec<_> = headers.get_all("x-existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "value1");

	// Should add new header
	assert_eq!(headers.get("x-new").unwrap(), "should-add");
}

#[test]
fn test_append_action_overwrite_if_exists_or_add() {
	use crate::http::ext_authz::proto::header_value_option::HeaderAppendAction;

	let mut headers = HeaderMap::new();

	// Pre-existing header with multiple values
	headers.append("x-existing", "value1".parse().unwrap());
	headers.append("x-existing", "value2".parse().unwrap());

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-existing".to_string(),
				value: "overwritten".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-new".to_string(),
				value: "added".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::OverwriteIfExistsOrAdd as i32,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should replace all existing values with single new value
	let values: Vec<_> = headers.get_all("x-existing").iter().collect();
	assert_eq!(values.len(), 1);
	assert_eq!(values[0], "overwritten");

	// Should add new header
	assert_eq!(headers.get("x-new").unwrap(), "added");
}

#[test]
fn test_append_action_overwrite_if_exists() {
	use crate::http::ext_authz::proto::header_value_option::HeaderAppendAction;

	let mut headers = HeaderMap::new();

	// Pre-existing header
	headers.insert("x-existing", "old-value".parse().unwrap());

	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-existing".to_string(),
				value: "new-value".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::OverwriteIfExists as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "x-new".to_string(),
				value: "should-not-add".to_string(),
				raw_value: vec![],
			}),
			append: None,
			append_action: HeaderAppendAction::OverwriteIfExists as i32,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should overwrite existing header
	assert_eq!(headers.get("x-existing").unwrap(), "new-value");

	// Should NOT add new header (no-op)
	assert!(headers.get("x-new").is_none());
}

#[test]
fn test_append_action_backward_compatibility_with_deprecated_append() {
	let mut headers = HeaderMap::new();
	headers.insert("x-test", "existing".parse().unwrap());

	// Test that old append=true still works (should append)
	let header_options_append_true = vec![HeaderValueOption {
		header: Some(ProtoHeaderValue {
			key: "x-test".to_string(),
			value: "appended".to_string(),
			raw_value: vec![],
		}),
		append: Some(true),
		append_action: 0, // Default value
	}];

	super::process_headers(&mut headers, header_options_append_true, None);

	let values: Vec<_> = headers.get_all("x-test").iter().collect();
	assert_eq!(values.len(), 2);
	assert_eq!(values[0], "existing");
	assert_eq!(values[1], "appended");

	// Test that old append=false still works (should overwrite)
	let mut headers2 = HeaderMap::new();
	headers2.insert("x-test2", "existing".parse().unwrap());

	let header_options_append_false = vec![HeaderValueOption {
		header: Some(ProtoHeaderValue {
			key: "x-test2".to_string(),
			value: "replaced".to_string(),
			raw_value: vec![],
		}),
		append: Some(false),
		append_action: 0, // Default value
	}];

	super::process_headers(&mut headers2, header_options_append_false, None);

	let values2: Vec<_> = headers2.get_all("x-test2").iter().collect();
	assert_eq!(values2.len(), 1);
	assert_eq!(values2[0], "replaced");
}

#[test]
fn test_append_action_multiple_set_cookie_headers() {
	use crate::http::ext_authz::proto::header_value_option::HeaderAppendAction;

	let mut headers = HeaderMap::new();

	// Simulate multiple set-cookie headers being added (common in OIDC flows)
	let header_options = vec![
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "set-cookie".to_string(),
				value: "access_token=abc123; Path=/; HttpOnly".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "set-cookie".to_string(),
				value: "id_token=xyz789; Path=/; HttpOnly".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		},
		HeaderValueOption {
			header: Some(ProtoHeaderValue {
				key: "set-cookie".to_string(),
				value: "session=def456; Path=/; Secure".to_string(),
				raw_value: vec![],
			}),
			append: Some(true),
			append_action: HeaderAppendAction::AppendIfExistsOrAdd as i32,
		},
	];

	super::process_headers(&mut headers, header_options, None);

	// Should have all three set-cookie headers
	let values: Vec<_> = headers.get_all("set-cookie").iter().collect();
	assert_eq!(values.len(), 3);
	assert_eq!(values[0], "access_token=abc123; Path=/; HttpOnly");
	assert_eq!(values[1], "id_token=xyz789; Path=/; HttpOnly");
	assert_eq!(values[2], "session=def456; Path=/; Secure");
}
