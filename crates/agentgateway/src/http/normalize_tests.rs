#[cfg(feature = "internal_benches")]
use crate::http::normalize_path;
#[cfg(test)]
use crate::http::{Body, Request, normalize_path_for_proxy, path_and_query_string};
#[cfg(test)]
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
	("noop", "/v1/chat/completions", false, false),
	("merge_slash_noop", "/v1/chat/completions", true, false),
	("dot_segment_noop", "/v1/chat/completions", false, true),
	("merge_slash_hit", "//v1//chat///completions", true, false),
	("dot_segment_hit", "/v1/../chat/./completions", false, true),
	("both_hit", "//v1/../chat/./completions", true, true),
])]
fn bench_normalize_path(
	b: Bencher,
	(name, path, apply_merge_slash, apply_dot_segment): (&str, &str, bool, bool),
) {
	let b = b.counter(divan::counter::BytesCount::new(path.len()));
	b.bench_local(|| {
		divan::black_box(name);
		let normalized = normalize_path(
			divan::black_box(path),
			divan::black_box(apply_merge_slash),
			divan::black_box(apply_dot_segment),
		);
		divan::black_box(normalized);
	});
}
