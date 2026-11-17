#[cfg(feature = "internal_benches")]
use divan::Bencher;

#[cfg(feature = "internal_benches")]
use crate::http::normalize_path;
#[cfg(test)]
use crate::http::{Body, Request, Uri, normalize_path_for_proxy};
#[cfg(test)]
use crate::proxy::ProxyError;
#[cfg(any(test, feature = "internal_benches"))]
use crate::types::frontend::PathNormalization;

#[cfg(test)]
fn request(uri: &str) -> Request {
	::http::Request::builder()
		.uri(format!("https://example.com{uri}"))
		.body(Body::empty())
		.expect("valid request")
}

#[cfg(test)]
fn path_and_query_string(uri: &Uri) -> String {
	uri
		.path_and_query()
		.map(|pq| pq.to_string())
		.unwrap_or_else(|| uri.path().to_string())
}

#[cfg(test)]
#[test]
fn path_normalization_table() {
	let merge_slash_cases: &[(&str, &str)] = &[
		("/", "/"),
		("//", "/"),
		("/foo/", "/foo/"),
		("/foo//", "/foo/"),
		("//foo///bar", "/foo/bar"),
		("/foo//bar?x=1", "/foo/bar?x=1"),
	];
	let dot_segment_cases: &[(&str, &str)] = &[
		("/foo/./bar", "/foo/bar"),
		("/foo/../bar", "/bar"),
		("/foo/./../bar", "/bar"),
		("/./foo/../bar?x=1&y=2", "/bar?x=1&y=2"),
		("/a/b/../../c", "/c"),
		("/a//b///c", "/a//b///c"),
		("/..", "/"),
		("/.", "/"),
		("/a/b/..", "/a/"),
		("/a/.", "/a/"),
		("/a/b/..?x=1", "/a/?x=1"),
		("/a.b/c..", "/a.b/c.."),
	];
	let both_cases: &[(&str, &str)] = &[("/foo", "/foo"), ("//foo/./../bar", "/bar"), ("/*", "/*")];

	use PathNormalization::*;
	let percent_cases: &[(&[PathNormalization], &str, Option<&str>)] = &[
		(&[DecodeUnreserved], "/%61dmin/", Some("/admin/")),
		(&[DecodeUnreserved], "/api/%6detrics", Some("/api/metrics")),
		(
			&[DecodeUnreserved],
			"/%41%7a%30%2D%2e%5F%7e",
			Some("/Az0-._~"),
		),
		(&[DecodeUnreserved], "/%d0%ba", Some("/%D0%BA")),
		(&[DecodeUnreserved], "/%2561", Some("/%2561")),
		(&[DecodeUnreserved], "/plain?q=%61", Some("/plain?q=%61")),
		(&[DecodeUnreserved], "/a%zz", None),
		(&[DecodeUnreserved], "/a%4", None),
		(
			&[DecodeUnreserved],
			"/api%2fmetrics",
			Some("/api%2Fmetrics"),
		),
		(
			&[DecodeUnreserved, DotSegment],
			"/foo/%2e%2E/admin",
			Some("/admin"),
		),
		(&[RejectEscapedSlash], "/api%2Fmetrics", None),
		(&[RejectEscapedSlash], "/api%5cmetrics", None),
		(&[RejectEscapedSlash], "/%61pi", Some("/%61pi")),
		(&[RejectEscapedSlash], "/%d0%ba", Some("/%D0%BA")),
		(&[RejectEscapedSlash], "/a%zz", None),
		(&[RejectEscapedSlash], "/a?x=%2F", Some("/a?x=%2F")),
		(&[RejectEscapedSlash, DotSegment], "/a%5C..%5Cadmin", None),
		(
			&[UnescapeSlash, DotSegment],
			"/a%5C..%5Cadmin",
			Some("/admin"),
		),
		(&[UnescapeSlash], "/api%2Fmetrics", Some("/api/metrics")),
		(&[UnescapeSlash], "/a%5Cb", Some("/a/b")),
		(
			&[UnescapeSlash, MergeSlash],
			"/api%2F%2fmetrics",
			Some("/api/metrics"),
		),
		(
			&[DecodeUnreserved, UnescapeSlash, DotSegment],
			"/a/..%2F..%2Fadmin",
			Some("/admin"),
		),
		(
			&[DecodeUnreserved, RejectEscapedSlash],
			"/plain/path",
			Some("/plain/path"),
		),
		(&[DotSegment], "/public/%2e%2E/admin", Some("/admin")),
		(
			&[DotSegment],
			"/public/%61/admin",
			Some("/public/%61/admin"),
		),
		(&[DotSegment], "/public\\..\\admin", Some("/admin")),
		(&[MergeSlash], "/a\\\\b", Some("/a/b")),
		(&[DotSegment], "/public/..;x=y/admin", Some("/admin")),
		(&[DotSegment], "/a/.;x/b;y", Some("/a/b;y")),
		(&[DecodeUnreserved], "/admin%00.css", None),
		(&[DotSegment], "/a%zz", None),
	];
	for (rules, input, expected) in percent_cases {
		let mut req = request(input);
		let res = normalize_path_for_proxy(&mut req, rules);
		match expected {
			Some(expected) => {
				res.unwrap_or_else(|e| panic!("{input}: {e}"));
				assert_eq!(path_and_query_string(req.uri()), *expected, "{input}");
			},
			None => assert!(
				matches!(res, Err(ProxyError::InvalidRequest)),
				"{input} should be rejected as invalid, got {res:?}"
			),
		}
	}

	for (input, expected) in merge_slash_cases {
		let mut req = request(input);
		normalize_path_for_proxy(&mut req, &[PathNormalization::MergeSlash])
			.expect("normalization should succeed");
		assert_eq!(path_and_query_string(req.uri()), *expected, "{input}");
	}

	for (input, expected) in dot_segment_cases {
		let mut req = request(input);
		normalize_path_for_proxy(&mut req, &[PathNormalization::DotSegment])
			.expect("normalization should succeed");
		assert_eq!(path_and_query_string(req.uri()), *expected, "{input}");
	}

	for (input, expected) in both_cases {
		let mut req = request(input);
		normalize_path_for_proxy(
			&mut req,
			&[PathNormalization::MergeSlash, PathNormalization::DotSegment],
		)
		.expect("normalization should succeed");
		assert_eq!(path_and_query_string(req.uri()), *expected, "{input}");
	}
}

#[cfg(test)]
#[test]
fn path_normalization_skips_non_origin_form() {
	use PathNormalization::*;
	let all = [MergeSlash, DotSegment, DecodeUnreserved, RejectEscapedSlash];

	let mut options = ::http::Request::builder()
		.method(::http::Method::OPTIONS)
		.uri("*")
		.body(Body::empty())
		.expect("valid request");
	normalize_path_for_proxy(&mut options, &all).expect("OPTIONS * is left alone");
	assert_eq!(options.uri(), "*");

	let mut connect = ::http::Request::builder()
		.method(::http::Method::CONNECT)
		.uri("example.com:443")
		.body(Body::empty())
		.expect("valid request");
	normalize_path_for_proxy(&mut connect, &all).expect("CONNECT is left alone");
	assert_eq!(connect.uri(), "example.com:443");
}

#[cfg(feature = "internal_benches")]
#[divan::bench(args = [
	("noop", "/v1/chat/completions", &[] as &[PathNormalization]),
	("merge_slash_noop", "/v1/chat/completions", &[PathNormalization::MergeSlash]),
	("dot_segment_noop", "/v1/chat/completions", &[PathNormalization::DotSegment]),
	("decode_noop", "/v1/chat/completions", &[PathNormalization::DecodeUnreserved]),
	("merge_slash_hit", "//v1//chat///completions", &[PathNormalization::MergeSlash]),
	("dot_segment_hit", "/v1/../chat/./completions", &[PathNormalization::DotSegment]),
	("decode_hit", "/v1/%63hat/completions", &[PathNormalization::DecodeUnreserved]),
	("both_hit", "//v1/../chat/./completions", &[PathNormalization::MergeSlash, PathNormalization::DotSegment]),
])]
fn bench_normalize_path(b: Bencher, (name, path, normalize): (&str, &str, &[PathNormalization])) {
	let b = b.counter(divan::counter::BytesCount::new(path.len()));
	b.bench_local(|| {
		divan::black_box(name);
		let normalized = normalize_path(divan::black_box(path), divan::black_box(normalize));
		let _ = divan::black_box(normalized);
	});
}
