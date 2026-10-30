#[cfg(feature = "internal_benches")]
use crate::http::normalize_path;
#[cfg(test)]
use crate::http::{Body, Request, normalize_path_for_proxy, path_and_query_string};
#[cfg(any(test, feature = "internal_benches"))]
use crate::types::frontend::PathNormalization;
#[cfg(feature = "internal_benches")]
use divan::Bencher;

#[cfg(test)]
fn request(uri: &str) -> Request {
	::http::Request::builder()
		.uri(format!("https://example.com{uri}"))
		.body(Body::empty())
		.expect("valid request")
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
		(&[UnescapeSlash], "/api%2Fmetrics", Some("/api/metrics")),
		(&[UnescapeSlash], "/a%5Cb", Some("/a\\b")),
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
	];
	for (rules, input, expected) in percent_cases {
		let mut req = request(input);
		let res = normalize_path_for_proxy(&mut req, rules);
		match expected {
			Some(expected) => {
				res.unwrap_or_else(|e| panic!("{input}: {e}"));
				assert_eq!(path_and_query_string(req.uri()), *expected, "{input}");
			},
			None => assert!(res.is_err(), "{input} should be rejected"),
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
