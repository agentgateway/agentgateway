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
