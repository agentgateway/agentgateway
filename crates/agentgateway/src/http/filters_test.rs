use std::num::NonZeroU16;

use regex;

use crate::http::StatusCode;
use crate::http::filters::{RequestRedirect, UrlRewrite};
use crate::http::tests_common::*;
use crate::types::agent::{HostRedirect, PathMatch, PathRedirect};
use crate::*;

#[test]
fn redirection_test() {
	#[derive(Debug, Eq, PartialEq)]
	struct Want {
		location: String,
		code: StatusCode,
	}
	struct Input<'a> {
		path: &'a PathMatch,
		redirect: &'a RequestRedirect,
		uri: &'a str,
	}

	let match_any = PathMatch::PathPrefix("/".into());
	let match_api = PathMatch::PathPrefix("/api".into());
	let match_api_slash = PathMatch::PathPrefix("/api/".into());
	let match_old = PathMatch::PathPrefix("/old".into());

	let no_redirect = RequestRedirect {
		scheme: None,
		authority: None,
		path: None,
		status: None,
	};

	let https_scheme = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTPS),
		authority: None,
		path: None,
		status: None,
	};

	let host_redirect = RequestRedirect {
		scheme: None,
		authority: Some(HostRedirect::Host("newhost.com".into())),
		path: None,
		status: None,
	};

	let full_authority = RequestRedirect {
		scheme: None,
		authority: Some(HostRedirect::Full("newhost.com:8080".into())),
		path: None,
		status: None,
	};

	let port_redirect = RequestRedirect {
		scheme: None,
		authority: Some(HostRedirect::Port(NonZeroU16::new(8080).unwrap())),
		path: None,
		status: None,
	};

	let full_path = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Full("/new/path".into())),
		status: None,
	};

	let prefix_path = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Prefix("/v1".into())),
		status: None,
	};

	let prefix_path_v2 = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Prefix("/v2".into())),
		status: None,
	};

	let prefix_path_v1_slash = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Prefix("/v1/".into())),
		status: None,
	};

	let prefix_path_slash = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Prefix("/".into())),
		status: None,
	};

	let prefix_path_empty = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Prefix("".into())),
		status: None,
	};

	let status_override = RequestRedirect {
		scheme: None,
		authority: None,
		path: None,
		status: Some(StatusCode::MOVED_PERMANENTLY),
	};

	let combined_redirect = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTPS),
		authority: Some(HostRedirect::Host("newhost.com".into())),
		path: Some(PathRedirect::Prefix("/new".into())),
		status: Some(StatusCode::PERMANENT_REDIRECT),
	};

	let http_port_80 = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTP),
		authority: Some(HostRedirect::Port(NonZeroU16::new(80).unwrap())),
		path: None,
		status: None,
	};

	let https_port_443 = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTPS),
		authority: Some(HostRedirect::Port(NonZeroU16::new(443).unwrap())),
		path: None,
		status: None,
	};

	let http_port_8080 = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTP),
		authority: Some(HostRedirect::Port(NonZeroU16::new(8080).unwrap())),
		path: None,
		status: None,
	};

	let https_port_8443 = RequestRedirect {
		scheme: Some(http::uri::Scheme::HTTPS),
		authority: Some(HostRedirect::Port(NonZeroU16::new(8443).unwrap())),
		path: None,
		status: None,
	};

	let redirect_307 = RequestRedirect {
		scheme: None,
		authority: None,
		path: None,
		status: Some(StatusCode::TEMPORARY_REDIRECT),
	};

	let match_exact = PathMatch::Exact("/exact".into());
	let exact_path = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Full("/new-exact".into())),
		status: None,
	};

	let match_regex = PathMatch::Regex(regex::Regex::new(r"^/regex/(\d+)$").unwrap(), 1);
	let regex_path = RequestRedirect {
		scheme: None,
		authority: None,
		path: Some(PathRedirect::Full("/new-regex".into())),
		status: None,
	};

	let cases = vec![
		// Basic test - no redirect configuration, should use original URI
		(
			"simple_no_redirect",
			Input {
				path: &match_any,
				redirect: &no_redirect,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://test.com/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test scheme redirect
		(
			"scheme_redirect_http_to_https",
			Input {
				path: &match_any,
				redirect: &https_scheme,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "https://test.com/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test host redirect
		(
			"host_redirect",
			Input {
				path: &match_any,
				redirect: &host_redirect,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://newhost.com/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test full authority redirect
		(
			"full_authority_redirect",
			Input {
				path: &match_any,
				redirect: &full_authority,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://newhost.com:8080/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test port redirect
		(
			"port_redirect",
			Input {
				path: &match_any,
				redirect: &port_redirect,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://test.com:8080/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path redirect - full path
		(
			"full_path_redirect",
			Input {
				path: &match_any,
				redirect: &full_path,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://test.com/new/path".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path redirect - prefix
		(
			"prefix_path_redirect",
			Input {
				path: &match_api,
				redirect: &prefix_path,
				uri: "http://test.com/api/users/123",
			},
			Some(Want {
				location: "http://test.com/v1/users/123".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test status code override
		(
			"status_code_override",
			Input {
				path: &match_any,
				redirect: &status_override,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://test.com/hello/world".to_string(),
				code: StatusCode::MOVED_PERMANENTLY,
			}),
		),
		// Test combined redirect - scheme, host, path, and status
		(
			"combined_redirect",
			Input {
				path: &match_old,
				redirect: &combined_redirect,
				uri: "http://test.com/old/api/users",
			},
			Some(Want {
				location: "https://newhost.com/new/api/users".to_string(),
				code: StatusCode::PERMANENT_REDIRECT,
			}),
		),
		// Test port handling with HTTP scheme (should omit port 80)
		(
			"http_port_80_omitted",
			Input {
				path: &match_any,
				redirect: &http_port_80,
				uri: "https://test.com:443/hello/world",
			},
			Some(Want {
				location: "http://test.com/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test port handling with HTTPS scheme (should omit port 443)
		(
			"https_port_443_omitted",
			Input {
				path: &match_any,
				redirect: &https_port_443,
				uri: "http://test.com:80/hello/world",
			},
			Some(Want {
				location: "https://test.com/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test non-standard port with HTTP scheme
		(
			"http_non_standard_port_included",
			Input {
				path: &match_any,
				redirect: &http_port_8080,
				uri: "https://test.com:443/hello/world",
			},
			Some(Want {
				location: "http://test.com:8080/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test non-standard port with HTTPS scheme
		(
			"https_non_standard_port_included",
			Input {
				path: &match_any,
				redirect: &https_port_8443,
				uri: "http://test.com:80/hello/world",
			},
			Some(Want {
				location: "https://test.com:8443/hello/world".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test query parameters preservation
		(
			"query_parameters_preserved",
			Input {
				path: &match_any,
				redirect: &no_redirect,
				uri: "http://test.com/hello/world?param1=value1&param2=value2",
			},
			Some(Want {
				location: "http://test.com/hello/world?param1=value1&param2=value2".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path prefix with query parameters
		(
			"path_prefix_with_query",
			Input {
				path: &match_api,
				redirect: &prefix_path_v2,
				uri: "http://test.com/api/users?page=1&limit=10",
			},
			Some(Want {
				location: "http://test.com/v2/users?page=1&limit=10".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path prefix edge case - exact match
		(
			"path_prefix_exact_match",
			Input {
				path: &match_api,
				redirect: &prefix_path,
				uri: "http://test.com/api",
			},
			Some(Want {
				location: "http://test.com/v1".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path prefix edge case - with trailing slash
		(
			"path_prefix_trailing_slash",
			Input {
				path: &match_api_slash,
				redirect: &prefix_path_v1_slash,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				location: "http://test.com/v1/users".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		(
			"path_prefix_without_trailing_slash_to_slash",
			Input {
				path: &match_api,
				redirect: &prefix_path_slash,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				location: "http://test.com/users".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		(
			"path_prefix_with_trailing_slash_to_empty",
			Input {
				path: &match_api_slash,
				redirect: &prefix_path_empty,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				location: "http://test.com/users".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test different status codes
		(
			"status_code_307",
			Input {
				path: &match_any,
				redirect: &redirect_307,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				location: "http://test.com/hello/world".to_string(),
				code: StatusCode::TEMPORARY_REDIRECT,
			}),
		),
		// Test exact path match
		(
			"exact_path_match",
			Input {
				path: &match_exact,
				redirect: &exact_path,
				uri: "http://test.com/exact",
			},
			Some(Want {
				location: "http://test.com/new-exact".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test regex path match
		(
			"regex_path_match",
			Input {
				path: &match_regex,
				redirect: &regex_path,
				uri: "http://test.com/regex/123",
			},
			Some(Want {
				location: "http://test.com/new-regex".to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test URI without scheme
		(
			"uri_without_scheme",
			Input {
				path: &match_any,
				redirect: &no_redirect,
				uri: "//test.com/hello/world",
			},
			None, // Should be rejected
		),
		// Test complex query parameters with special characters
		(
			"complex_query_parameters",
			Input {
				path: &match_any,
				redirect: &no_redirect,
				uri: "http://test.com/hello/world?param1=value%201&param2=value%2B2&param3=value%3D3",
			},
			Some(Want {
				location: "http://test.com/hello/world?param1=value%201&param2=value%2B2&param3=value%3D3"
					.to_string(),
				code: StatusCode::FOUND,
			}),
		),
		// Test path prefix with empty rest
		(
			"path_prefix_empty_rest",
			Input {
				path: &match_api,
				redirect: &prefix_path,
				uri: "http://test.com/api",
			},
			Some(Want {
				location: "http://test.com/v1".to_string(),
				code: StatusCode::FOUND,
			}),
		),
	];
	for (name, inp, want) in cases {
		let mut req = request_for_uri(inp.uri);

		let got = inp
			.redirect
			.apply(&mut req, inp.path)
			.map(|resp| Want {
				location: resp.hdr(http::header::LOCATION).to_string(),
				code: resp.status(),
			})
			.ok();
		assert_eq!(got, want, "{name}");
	}
}

#[test]
fn rewrite_test() {
	#[derive(Debug, Eq, PartialEq)]
	struct Want {
		uri: String,
	}
	struct Input<'a> {
		path: &'a PathMatch,
		rewrite: &'a UrlRewrite,
		uri: &'a str,
	}

	let match_any = PathMatch::PathPrefix("/".into());
	let match_api = PathMatch::PathPrefix("/api".into());
	let match_api_slash = PathMatch::PathPrefix("/api/".into());
	let match_old = PathMatch::PathPrefix("/old".into());

	let no_rewrite = UrlRewrite {
		authority: None,
		path: None,
	};

	let host_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Host("newhost.com".into())),
		path: None,
	};

	let port_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Port(NonZeroU16::new(8080).unwrap())),
		path: None,
	};

	let full_path_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Full("/new/path".into())),
	};

	let prefix_path_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Prefix("/v1".into())),
	};

	let prefix_path_v2_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Prefix("/v2".into())),
	};

	let prefix_path_v1_slash_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Prefix("/v1/".into())),
	};

	let prefix_path_slash_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Prefix("/".into())),
	};

	let prefix_path_empty_rewrite = UrlRewrite {
		authority: None,
		path: Some(PathRedirect::Prefix("".into())),
	};

	let combined_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Host("newhost.com".into())),
		path: Some(PathRedirect::Prefix("/new".into())),
	};

	let http_port_80_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Port(NonZeroU16::new(80).unwrap())),
		path: None,
	};

	let https_port_443_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Port(NonZeroU16::new(443).unwrap())),
		path: None,
	};

	let http_port_8080_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Port(NonZeroU16::new(8080).unwrap())),
		path: None,
	};

	let https_port_8443_rewrite = UrlRewrite {
		authority: Some(HostRedirect::Port(NonZeroU16::new(8443).unwrap())),
		path: None,
	};

	let cases = vec![
		// Basic test - no rewrite configuration, should keep original URI
		(
			"simple_no_rewrite",
			Input {
				path: &match_any,
				rewrite: &no_rewrite,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				uri: "http://test.com/hello/world".to_string(),
			}),
		),
		// Test hostname rewrite
		(
			"hostname_rewrite",
			Input {
				path: &match_any,
				rewrite: &host_rewrite,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				uri: "http://newhost.com/hello/world".to_string(),
			}),
		),
		// Test port rewrite
		(
			"port_rewrite",
			Input {
				path: &match_any,
				rewrite: &port_rewrite,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				uri: "http://test.com:8080/hello/world".to_string(),
			}),
		),
		// Test full path rewrite
		(
			"full_path_rewrite",
			Input {
				path: &match_any,
				rewrite: &full_path_rewrite,
				uri: "http://test.com/hello/world",
			},
			Some(Want {
				uri: "http://test.com/new/path".to_string(),
			}),
		),
		// Test prefix path rewrite
		(
			"prefix_path_rewrite",
			Input {
				path: &match_api,
				rewrite: &prefix_path_rewrite,
				uri: "http://test.com/api/users/123",
			},
			Some(Want {
				uri: "http://test.com/v1/users/123".to_string(),
			}),
		),
		// Test combined rewrite - hostname and path
		(
			"combined_rewrite",
			Input {
				path: &match_old,
				rewrite: &combined_rewrite,
				uri: "http://test.com/old/api/users",
			},
			Some(Want {
				uri: "http://newhost.com/new/api/users".to_string(),
			}),
		),
		// Test port handling with HTTP scheme (should omit port 80)
		(
			"http_port_80_omitted",
			Input {
				path: &match_any,
				rewrite: &http_port_80_rewrite,
				uri: "https://test.com:443/hello/world",
			},
			Some(Want {
				uri: "https://test.com:80/hello/world".to_string(),
			}),
		),
		// Test port handling with HTTPS scheme (should omit port 443)
		(
			"https_port_443_omitted",
			Input {
				path: &match_any,
				rewrite: &https_port_443_rewrite,
				uri: "http://test.com:80/hello/world",
			},
			Some(Want {
				uri: "http://test.com:443/hello/world".to_string(),
			}),
		),
		// Test non-standard port with HTTP scheme
		(
			"http_non_standard_port_included",
			Input {
				path: &match_any,
				rewrite: &http_port_8080_rewrite,
				uri: "https://test.com:443/hello/world",
			},
			Some(Want {
				uri: "https://test.com:8080/hello/world".to_string(),
			}),
		),
		// Test non-standard port with HTTPS scheme
		(
			"https_non_standard_port_included",
			Input {
				path: &match_any,
				rewrite: &https_port_8443_rewrite,
				uri: "http://test.com:80/hello/world",
			},
			Some(Want {
				uri: "http://test.com:8443/hello/world".to_string(),
			}),
		),
		// Test query parameters preservation
		(
			"query_parameters_preserved",
			Input {
				path: &match_any,
				rewrite: &no_rewrite,
				uri: "http://test.com/hello/world?param1=value1&param2=value2",
			},
			Some(Want {
				uri: "http://test.com/hello/world?param1=value1&param2=value2".to_string(),
			}),
		),
		// Test path prefix with query parameters
		(
			"path_prefix_with_query",
			Input {
				path: &match_api,
				rewrite: &prefix_path_v2_rewrite,
				uri: "http://test.com/api/users?page=1&limit=10",
			},
			Some(Want {
				uri: "http://test.com/v2/users?page=1&limit=10".to_string(),
			}),
		),
		// Test path prefix edge case - exact match
		(
			"path_prefix_exact_match",
			Input {
				path: &match_api,
				rewrite: &prefix_path_rewrite,
				uri: "http://test.com/api",
			},
			Some(Want {
				uri: "http://test.com/v1".to_string(),
			}),
		),
		// Test path prefix edge case - with trailing slash
		(
			"path_prefix_trailing_slash",
			Input {
				path: &match_api_slash,
				rewrite: &prefix_path_v1_slash_rewrite,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				uri: "http://test.com/v1/users".to_string(),
			}),
		),
		// Test path prefix edge case - replace prefix_without_trailing_slash with /
		(
			"path_prefix_trailing_slash",
			Input {
				path: &match_api,
				rewrite: &prefix_path_slash_rewrite,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				uri: "http://test.com/users".to_string(),
			}),
		),
		// Test path prefix edge case - replace prefix_with_trailing_slash with empty
		(
			"path_prefix_trailing_slash",
			Input {
				path: &match_api_slash,
				rewrite: &prefix_path_empty_rewrite,
				uri: "http://test.com/api/users",
			},
			Some(Want {
				uri: "http://test.com/users".to_string(),
			}),
		),
		// Test complex query parameters with special characters
		(
			"complex_query_parameters",
			Input {
				path: &match_any,
				rewrite: &no_rewrite,
				uri: "http://test.com/hello/world?param1=value%201&param2=value%2B2&param3=value%3D3",
			},
			Some(Want {
				uri: "http://test.com/hello/world?param1=value%201&param2=value%2B2&param3=value%3D3"
					.to_string(),
			}),
		),
		// Test path prefix with empty rest
		(
			"path_prefix_empty_rest",
			Input {
				path: &match_api,
				rewrite: &prefix_path_rewrite,
				uri: "http://test.com/api",
			},
			Some(Want {
				uri: "http://test.com/v1".to_string(),
			}),
		),
		// Test hostname rewrite with HTTPS
		(
			"hostname_rewrite_https",
			Input {
				path: &match_any,
				rewrite: &host_rewrite,
				uri: "https://test.com/hello/world",
			},
			Some(Want {
				uri: "https://newhost.com/hello/world".to_string(),
			}),
		),
		// Test hostname rewrite with custom port
		(
			"hostname_rewrite_custom_port",
			Input {
				path: &match_any,
				rewrite: &host_rewrite,
				uri: "http://test.com:8080/hello/world",
			},
			Some(Want {
				uri: "http://newhost.com/hello/world".to_string(),
			}),
		),
		// Test full path rewrite with query parameters - should preserve query params
		(
			"full_path_rewrite_with_query",
			Input {
				path: &match_any,
				rewrite: &full_path_rewrite,
				uri: "http://test.com/hello/world?param1=value1&param2=value2",
			},
			Some(Want {
				uri: "http://test.com/new/path?param1=value1&param2=value2".to_string(),
			}),
		),
		// Test hostname rewrite with query parameters - should preserve query params
		(
			"hostname_rewrite_with_query",
			Input {
				path: &match_any,
				rewrite: &host_rewrite,
				uri: "http://test.com/hello/world?param1=value1&param2=value2",
			},
			Some(Want {
				uri: "http://newhost.com/hello/world?param1=value1&param2=value2".to_string(),
			}),
		),
		// Test port rewrite with query parameters - should preserve query params
		(
			"port_rewrite_with_query",
			Input {
				path: &match_any,
				rewrite: &port_rewrite,
				uri: "http://test.com/hello/world?param1=value1&param2=value2",
			},
			Some(Want {
				uri: "http://test.com:8080/hello/world?param1=value1&param2=value2".to_string(),
			}),
		),
		// Test combined rewrite with query parameters - should preserve query params
		(
			"combined_rewrite_with_query",
			Input {
				path: &match_old,
				rewrite: &combined_rewrite,
				uri: "http://test.com/old/api/users?page=1&limit=10",
			},
			Some(Want {
				uri: "http://newhost.com/new/api/users?page=1&limit=10".to_string(),
			}),
		),
		// Test full path rewrite with complex query parameters
		(
			"full_path_rewrite_complex_query",
			Input {
				path: &match_any,
				rewrite: &full_path_rewrite,
				uri: "http://test.com/hello/world?param1=value%201&param2=value%2B2&param3=value%3D3",
			},
			Some(Want {
				uri: "http://test.com/new/path?param1=value%201&param2=value%2B2&param3=value%3D3"
					.to_string(),
			}),
		),
	];
	for (name, inp, want) in cases {
		let mut req = request_for_uri(inp.uri);

		let got = inp
			.rewrite
			.apply(&mut req, inp.path)
			.map(|_| Want {
				uri: req.uri().to_string(),
			})
			.ok();
		assert_eq!(got, want, "{name}");
	}
}
