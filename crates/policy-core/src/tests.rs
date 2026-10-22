use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

const TRACE_POLICY_KIND: &str = "test";

#[test]
fn noop_trace_does_not_render_details() {
	let rendered = AtomicBool::new(false);

	pol_event!(TraceSeverity::Info, "{}", {
		rendered.store(true, Ordering::Relaxed);
		"details"
	});

	assert!(!rendered.load(Ordering::Relaxed));
}

#[test]
fn test_properties() {
	let test = |e: &str, want: &[&str]| {
		let p = cel::Program::compile(e).unwrap();
		let mut props = Vec::with_capacity(5);
		crate::properties::properties(&p.expression().expr, &mut props, &mut Vec::default());
		let want = HashSet::from_iter(want.iter().map(|s| s.to_string()));
		let got = props
			.into_iter()
			.map(|p| p.join("."))
			.collect::<HashSet<_>>();
		assert_eq!(want, got, "expression: {e}");
	};

	test(r#"foo.bar.baz"#, &["foo.bar.baz"]);
	test(r#"foo["bar"]"#, &["foo"]);
	test(r#"foo.baz["bar"]"#, &["foo.baz"]);
	// This is not quite right but maybe good enough.
	test(r#"foo.with(x, x.body)"#, &["foo", "x", "x.body"]);
	test(r#"foo.map(x, x.body)"#, &["foo", "x", "x.body"]);
	test(r#"foo.bar.map(x, x.body)"#, &["foo.bar", "x", "x.body"]);

	test(r#"fn(bar.baz)"#, &["bar.baz"]);
	test(r#"{"key":val, "listkey":[a.b]}"#, &["val", "a.b"]);
	test(r#"{"key":val, "listkey":[a.b]}"#, &["val", "a.b"]);
	test(r#"a? b: c"#, &["a", "b", "c"]);
	test(r#"a || b"#, &["a", "b"]);
	test(r#"!a.b"#, &["a.b"]);
	test(r#"a.b < c"#, &["a.b", "c"]);
	test(r#"a.b + c + 2"#, &["a.b", "c"]);
	test(r#"a["b"].c"#, &["a"]);
	test(r#"a["b"]["c"]"#, &["a"]);
	test(r#"a.b[0]"#, &["a.b"]);
	test(r#"a.b[0].c"#, &["a.b"]);
	test(r#"a[b.c]"#, &["a", "b.c"]);
	test(r#"{"a":"b"}.a"#, &[]);
	// Test extauthz namespace recognition
	test(r#"extauthz.user_id"#, &["extauthz.user_id"]);
	test(r#"extauthz.role == "admin""#, &["extauthz.role"]);
}
