use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use agent_core::strng;
use divan::Bencher;
use itertools::Itertools;
use regex::Regex;

use crate::http::Request;
use crate::http::tests_common::*;
use crate::store::Stores;
use crate::types::agent::{
	HeaderMatch, HeaderValueMatch, Listener, ListenerProtocol, MethodMatch, PathMatch, QueryMatch,
	QueryValueMatch, Route, RouteMatch, RouteSet,
};
use crate::*;

fn run_test(req: &Request, routes: &[(&str, Vec<&str>, Vec<RouteMatch>)]) -> Option<String> {
	let stores = Stores::from_init(crate::store::StoresInit::default());
	let network = strng::literal!("network");
	let dummy_dest = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1000);

	let listener = setup_listener(routes);

	let result = super::select_best_route(
		stores.clone(),
		network.clone(),
		None,
		dummy_dest,
		&listener,
		req,
	);
	result.map(|(r, _)| r.key.to_string())
}

fn setup_listener(routes: &[(&str, Vec<&str>, Vec<RouteMatch>)]) -> Arc<Listener> {
	let mk_route = |name: &str, hostnames: Vec<&str>, matches: Vec<RouteMatch>| Route {
		key: name.into(),
		hostnames: hostnames.into_iter().map(|s| s.into()).collect(),
		matches,
		name: Default::default(),
		backends: vec![],
		inline_policies: vec![],
	};

	Arc::new(Listener {
		key: Default::default(),
		name: Default::default(),
		hostname: Default::default(),
		protocol: ListenerProtocol::HTTP,
		tcp_routes: Default::default(),
		routes: RouteSet::from_list(
			routes
				.iter()
				.map(|r| {
					let r = r.clone();
					mk_route(r.0, r.1, r.2)
				})
				.collect(),
		),
	})
}

#[test]
fn test_hostname_matching() {
	let basic_match = vec![RouteMatch {
		headers: vec![],
		path: PathMatch::PathPrefix("/".into()),
		method: None,
		query: vec![],
	}];
	let routes = vec![
		// Route with no hostnames (matches any hostname)
		("no-hostnames", vec![], basic_match.clone()),
		// Route with exact hostname match
		(
			"exact-hostname",
			vec!["test.example.com"],
			basic_match.clone(),
		),
		// Route with wildcard hostname
		(
			"wildcard-hostname",
			vec!["*.example.com"],
			basic_match.clone(),
		),
		// Route with multiple hostnames
		(
			"multiple-hostnames",
			vec!["foo.example.com", "*.bar.example.com"],
			basic_match.clone(),
		),
	];

	struct TestCase {
		name: &'static str,
		host: &'static str,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		// Test exact hostname matching
		TestCase {
			name: "exact hostname match",
			host: "test.example.com",
			expected_route: Some("exact-hostname"),
		},
		// Test wildcard hostname matching
		TestCase {
			name: "wildcard hostname match - subdomain",
			host: "sub.example.com",
			expected_route: Some("wildcard-hostname"),
		},
		TestCase {
			name: "wildcard hostname match - nested subdomain",
			host: "foo.baz.example.com",
			expected_route: Some("wildcard-hostname"),
		},
		// Test multiple hostnames in route
		TestCase {
			name: "multiple hostnames - exact match",
			host: "foo.example.com",
			expected_route: Some("multiple-hostnames"),
		},
		TestCase {
			name: "multiple hostnames - wildcard match",
			host: "test.bar.example.com",
			// this also matches 'wildcard' but this one is a more exact match
			expected_route: Some("multiple-hostnames"),
		},
		// Test no hostnames route (should match any hostname)
		TestCase {
			name: "no hostnames route matches any hostname",
			host: "unknown",
			expected_route: Some("no-hostnames"),
		},
	];

	for case in cases {
		let req = request(&format!("http://{}/", case.host), http::Method::GET, &[]);
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_path_matching() {
	let routes = vec![
		("exact-path", PathMatch::Exact("/api/v1/users".into())),
		("prefix-path", PathMatch::PathPrefix("/api/".into())),
		(
			"regex-path",
			PathMatch::Regex(Regex::new(r"^/api/v\d+/users$").unwrap(), 18),
		),
		("root-prefix", PathMatch::PathPrefix("/".into())),
	];

	struct TestCase {
		name: &'static str,
		path: &'static str,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		// Test exact path matching
		TestCase {
			name: "exact path match",
			path: "/api/v1/users",
			expected_route: Some("exact-path"),
		},
		TestCase {
			// TODO: is this right?
			name: "exact path with trailing slash should not match",
			path: "/api/v1/users/",
			expected_route: Some("prefix-path"),
		},
		// Test prefix path matching
		TestCase {
			name: "prefix path match",
			path: "/api/blah/users",
			expected_route: Some("prefix-path"),
		},
		TestCase {
			name: "prefix path match with subpath",
			path: "/api/v1/users/123",
			expected_route: Some("prefix-path"),
		},
		// Test regex path matching
		TestCase {
			name: "regex path match",
			path: "/api/v2/users",
			expected_route: Some("regex-path"),
		},
		TestCase {
			name: "regex path match v3",
			path: "/api/v3/users",
			expected_route: Some("regex-path"),
		},
		// Test root prefix fallback
		TestCase {
			name: "root prefix fallback",
			path: "/other/path",
			expected_route: Some("root-prefix"),
		},
	];

	for case in cases {
		let req = request(
			&format!("http://example.com{}", case.path),
			http::Method::GET,
			&[],
		);
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, pm)| {
				(
					name,
					vec![],
					vec![RouteMatch {
						headers: vec![],
						path: pm.clone(),
						method: None,
						query: vec![],
					}],
				)
			})
			.collect_vec();
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_method_matching() {
	let routes = vec![
		(
			"get-only",
			Some(MethodMatch {
				method: "GET".into(),
			}),
		),
		(
			"post-only",
			Some(MethodMatch {
				method: "POST".into(),
			}),
		),
		("any-method", None),
	];

	struct TestCase {
		name: &'static str,
		method: http::Method,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		TestCase {
			name: "GET method matches get-only route",
			method: http::Method::GET,
			expected_route: Some("get-only"),
		},
		TestCase {
			name: "POST method matches post-only route",
			method: http::Method::POST,
			expected_route: Some("post-only"),
		},
		TestCase {
			name: "PUT method matches any-method route",
			method: http::Method::PUT,
			expected_route: Some("any-method"),
		},
		TestCase {
			name: "DELETE method matches any-method route",
			method: http::Method::DELETE,
			expected_route: Some("any-method"),
		},
	];

	for case in cases {
		let req = request("http://example.com/", case.method, &[]);
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, mm)| {
				(
					name,
					vec![],
					vec![RouteMatch {
						headers: vec![],
						path: PathMatch::PathPrefix("/".into()),
						method: mm,
						query: vec![],
					}],
				)
			})
			.collect_vec();
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_header_matching() {
	let routes = vec![
		("no-headers", vec![]),
		(
			"exact-header",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("content-type")),
				value: HeaderValueMatch::Exact(http::HeaderValue::from_static("application/json")),
			}],
		),
		(
			"regex-header",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("user-agent")),
				value: HeaderValueMatch::Regex(Regex::new(r"^Mozilla/.*$").unwrap()),
			}],
		),
		(
			"multiple-headers",
			vec![
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("content-type")),
					value: HeaderValueMatch::Exact(http::HeaderValue::from_static("application/json")),
				},
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("authorization")),
					value: HeaderValueMatch::Regex(Regex::new(r"^Bearer .*$").unwrap()),
				},
			],
		),
	];

	struct TestCase {
		name: &'static str,
		headers: Vec<(&'static str, &'static str)>,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		TestCase {
			name: "no headers matches no-headers route",
			headers: vec![],
			expected_route: Some("no-headers"),
		},
		TestCase {
			name: "exact header match",
			headers: vec![("content-type", "application/json")],
			expected_route: Some("exact-header"),
		},
		TestCase {
			name: "regex header match",
			headers: vec![(
				"user-agent",
				"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
			)],
			expected_route: Some("regex-header"),
		},
		TestCase {
			name: "multiple headers match",
			headers: vec![
				("content-type", "application/json"),
				("authorization", "Bearer token123"),
			],
			expected_route: Some("multiple-headers"),
		},
		TestCase {
			name: "header mismatch returns no match",
			headers: vec![("content-type", "text/html")],
			expected_route: Some("no-headers"),
		},
	];

	for case in cases {
		let req = request("http://example.com/", http::Method::GET, &case.headers);
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, hm)| {
				(
					name,
					vec![],
					vec![RouteMatch {
						headers: hm,
						path: PathMatch::PathPrefix("/".into()),
						method: None,
						query: vec![],
					}],
				)
			})
			.collect_vec();
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_pseudo_header_matching() {
	let routes = vec![
		("no-headers", vec![]),
		(
			"authority-exact",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Authority,
				value: HeaderValueMatch::Exact(http::HeaderValue::from_static("api.example.com")),
			}],
		),
		(
			"authority-regex",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Authority,
				value: HeaderValueMatch::Regex(Regex::new(r"^.*\.example\.com$").unwrap()),
			}],
		),
		(
			"method-post",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Method,
				value: HeaderValueMatch::Exact(http::HeaderValue::from_static("POST")),
			}],
		),
		(
			"scheme-https",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Scheme,
				value: HeaderValueMatch::Exact(http::HeaderValue::from_static("https")),
			}],
		),
		(
			"path-regex",
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Path,
				value: HeaderValueMatch::Regex(Regex::new(r"^/api/.*$").unwrap()),
			}],
		),
		(
			"multiple-pseudo",
			vec![
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Authority,
					value: HeaderValueMatch::Exact(http::HeaderValue::from_static("api.example.com")),
				},
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Method,
					value: HeaderValueMatch::Exact(http::HeaderValue::from_static("POST")),
				},
			],
		),
	];

	struct TestCase {
		name: &'static str,
		url: &'static str,
		method: http::Method,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		TestCase {
			name: "no pseudo headers matches no-headers route",
			url: "http://example.com/",
			method: http::Method::GET,
			expected_route: Some("no-headers"),
		},
		TestCase {
			name: "exact authority match",
			url: "http://api.example.com/",
			method: http::Method::GET,
			expected_route: Some("authority-exact"),
		},
		TestCase {
			name: "regex authority match",
			url: "http://test.example.com/",
			method: http::Method::GET,
			expected_route: Some("authority-regex"),
		},
		TestCase {
			name: "method POST match",
			url: "http://example.com/",
			method: http::Method::POST,
			expected_route: Some("method-post"),
		},
		TestCase {
			name: "scheme https match",
			url: "https://example.com/",
			method: http::Method::GET,
			expected_route: Some("scheme-https"),
		},
		TestCase {
			name: "path regex match",
			url: "http://example.com/api/users",
			method: http::Method::GET,
			expected_route: Some("path-regex"),
		},
		TestCase {
			name: "multiple pseudo headers match",
			url: "http://api.example.com/",
			method: http::Method::POST,
			expected_route: Some("multiple-pseudo"),
		},
		TestCase {
			name: "authority mismatch returns no match",
			url: "http://other.com/",
			method: http::Method::GET,
			expected_route: Some("no-headers"),
		},
	];

	for case in cases {
		let req = request(case.url, case.method, &[]);
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, hm)| {
				(
					name,
					vec![],
					vec![RouteMatch {
						headers: hm,
						path: PathMatch::PathPrefix("/".into()),
						method: None,
						query: vec![],
					}],
				)
			})
			.collect_vec();
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_query_parameter_matching() {
	let routes = vec![
		("no-query", vec![]),
		(
			"exact-query",
			vec![QueryMatch {
				name: "version".into(),
				value: QueryValueMatch::Exact("v1".into()),
			}],
		),
		(
			"regex-query",
			vec![QueryMatch {
				name: "id".into(),
				value: QueryValueMatch::Regex(Regex::new(r"^\d+$").unwrap()),
			}],
		),
		(
			"multiple-query",
			vec![
				QueryMatch {
					name: "version".into(),
					value: QueryValueMatch::Exact("v2".into()),
				},
				QueryMatch {
					name: "format".into(),
					value: QueryValueMatch::Exact("json".into()),
				},
			],
		),
	];

	struct TestCase {
		name: &'static str,
		query: &'static str,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		TestCase {
			name: "no query parameters matches no-query route",
			query: "",
			expected_route: Some("no-query"),
		},
		TestCase {
			name: "exact query parameter match",
			query: "version=v1",
			expected_route: Some("exact-query"),
		},
		TestCase {
			name: "regex query parameter match",
			query: "id=123",
			expected_route: Some("regex-query"),
		},
		TestCase {
			name: "multiple query parameters match",
			query: "version=v2&format=json",
			expected_route: Some("multiple-query"),
		},
		TestCase {
			name: "query parameter mismatch returns no match",
			query: "version=v3",
			expected_route: Some("no-query"),
		},
		TestCase {
			name: "regex query parameter mismatch",
			query: "id=abc",
			expected_route: Some("no-query"),
		},
	];

	for case in cases {
		let uri = if case.query.is_empty() {
			"http://example.com/".to_string()
		} else {
			format!("http://example.com/?{}", case.query)
		};
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, qm)| {
				(
					name,
					vec![],
					vec![RouteMatch {
						headers: vec![],
						path: PathMatch::PathPrefix("/".into()),
						method: None,
						query: qm,
					}],
				)
			})
			.collect_vec();
		let req = request(&uri, http::Method::GET, &[]);
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[test]
fn test_route_precedence() {
	let routes = vec![
		// Route with exact hostname (should have higher precedence than wildcard)
		(
			"exact-hostname-exact-path",
			vec!["test.example.com"],
			PathMatch::Exact("/api/users".into()),
			None,
			vec![],
		),
		(
			"wildcard-hostname-exact-path",
			vec!["*.example.com"],
			PathMatch::Exact("/api/users".into()),
			None,
			vec![],
		),
		// Route with longer prefix path (should have higher precedence)
		(
			"longer-prefix",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/users/".into()),
			None,
			vec![],
		),
		(
			"shorter-prefix",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/".into()),
			None,
			vec![],
		),
		// Route with method match (should have higher precedence than no method)
		(
			"with-method",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/".into()),
			Some(MethodMatch {
				method: "GET".into(),
			}),
			vec![],
		),
		(
			"without-method",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/".into()),
			None,
			vec![],
		),
		// Route with more header matches (should have higher precedence)
		(
			"more-headers",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/".into()),
			None,
			vec![
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("content-type")),
					value: HeaderValueMatch::Exact(http::HeaderValue::from_static("application/json")),
				},
				HeaderMatch {
					name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("authorization")),
					value: HeaderValueMatch::Exact(http::HeaderValue::from_static("Bearer token")),
				},
			],
		),
		(
			"fewer-headers",
			vec!["test.example.com"],
			PathMatch::PathPrefix("/api/".into()),
			None,
			vec![HeaderMatch {
				name: crate::http::HeaderOrPseudo::Header(http::HeaderName::from_static("content-type")),
				value: HeaderValueMatch::Exact(http::HeaderValue::from_static("application/json")),
			}],
		),
	];

	struct TestCase {
		name: &'static str,
		host: &'static str,
		path: &'static str,
		method: http::Method,
		headers: Vec<(&'static str, &'static str)>,
		expected_route: Option<&'static str>,
	}

	let cases = vec![
		// Test hostname precedence: exact over wildcard
		TestCase {
			name: "exact hostname takes precedence over wildcard",
			host: "test.example.com",
			path: "/api/users",
			method: http::Method::GET,
			headers: vec![],
			expected_route: Some("exact-hostname-exact-path"),
		},
		// Test path precedence: longer prefix over shorter
		TestCase {
			name: "longer path prefix takes precedence",
			host: "test.example.com",
			path: "/api/users/123",
			method: http::Method::GET,
			headers: vec![],
			expected_route: Some("longer-prefix"),
		},
		// Test method precedence: with method over without
		TestCase {
			name: "method match takes precedence over no method",
			host: "test.example.com",
			path: "/api/other",
			method: http::Method::GET,
			headers: vec![],
			expected_route: Some("with-method"),
		},
		// Test header precedence: more headers over fewer
		TestCase {
			name: "more header matches takes precedence",
			host: "test.example.com",
			path: "/api/other",
			method: http::Method::POST,
			headers: vec![
				("content-type", "application/json"),
				("authorization", "Bearer token"),
			],
			expected_route: Some("more-headers"),
		},
	];

	for case in cases {
		let uri = format!("http://{}{}", case.host, case.path);
		let req = request(&uri, case.method, &case.headers);
		let routes = routes
			.clone()
			.into_iter()
			.map(|(name, host, path, method, headers)| {
				(
					name,
					host,
					vec![RouteMatch {
						headers,
						path,
						method,
						query: vec![],
					}],
				)
			})
			.collect_vec();
		let result = run_test(&req, routes.as_slice());
		assert_eq!(
			result,
			case.expected_route.map(|s| s.to_string()),
			"{}",
			case.name
		);
	}
}

#[divan::bench(args = [(1,1), (100, 100), (5000,100)])]
fn bench(b: Bencher, (host, route): (u64, u64)) {
	let mut routes = vec![];
	for host in 0..host {
		for path in 0..route {
			let m = vec![RouteMatch {
				headers: vec![],
				path: PathMatch::PathPrefix(strng::literal!("/{path}")),
				method: None,
				query: vec![],
			}];
			routes.push((
				format!("{host}-{path}"),
				vec![format!("{}", host)],
				m.clone(),
			));
		}
	}

	let listener = Arc::new(Listener {
		key: Default::default(),
		name: Default::default(),
		hostname: Default::default(),
		protocol: ListenerProtocol::HTTP,
		tcp_routes: Default::default(),
		routes: RouteSet::from_list(
			routes
				.into_iter()
				.map(|(name, host, matches)| Route {
					key: name.into(),
					name: Default::default(),
					hostnames: host.into_iter().map(|s| s.into()).collect(),
					matches,
					backends: vec![],
					inline_policies: vec![],
				})
				.collect(),
		),
	});
	let stores = Stores::from_init(crate::store::StoresInit::default());
	let network = strng::literal!("network");
	let dummy_dest = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1000);
	let req = request("http://example.com", http::Method::GET, &[]);

	b.bench_local(|| {
		divan::black_box(super::select_best_route(
			stores.clone(),
			network.clone(),
			None,
			dummy_dest,
			&listener,
			divan::black_box(&req),
		))
	});
}
