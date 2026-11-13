//! Reading a prefix of a conversation should not allocate for its unread text.

#![allow(clippy::unwrap_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use agent_llm_ir::{
	ClientProtocol, IdentifierPseudonymizer, StateScope, TextPlace, Upstream, UpstreamProtocol,
	prepare,
};

#[global_allocator]
static ALLOCATOR: CountAllocations = CountAllocations;

thread_local! {
		static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

#[test]
fn text_iteration_only_decodes_the_strings_it_reads() {
	let body = br#"{"model":"m","max_tokens":8,"system":"system","messages":[
        {"role":"assistant","content":[{"type":"tool_use","id":"c","name":"f","input":{"plain":"plain","first":"a\nb","second":"c\nd"}}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"c","content":"result"}]}
    ]}"#;
	let request = prepare(
		body,
		ClientProtocol::Messages,
		Upstream {
			identifier_pseudonymizer: &NoReplay,
			scope: StateScope {
				backend: "a",
				provider: "p",
				model: "m",
			},
			protocol: UpstreamProtocol::Responses {
				safety_identifier: false,
				reasoning_context: false,
			},
			default_max_tokens: None,
		},
	)
	.unwrap();
	let conversation = request.conversation();

	let (mut texts, allocations) = counted(|| conversation.texts());
	assert_eq!(
		allocations, 0,
		"constructing the iterator must not collect text"
	);
	let (system, allocations) = counted(|| texts.next().unwrap());
	assert_eq!(
		allocations, 0,
		"reading system text must not decode later arguments"
	);
	assert_eq!(system.0, TextPlace::System(0));
	assert_eq!(system.1.read(), "system");

	let (plain, allocations) = counted(|| texts.next().unwrap());
	assert_eq!(
		allocations, 0,
		"reading a plain argument must not collect or decode later strings"
	);
	assert!(matches!(plain.0, TextPlace::ToolCall { string: 0, .. }));
	assert_eq!(plain.1.read(), "plain");

	let (first, allocations) = counted(|| texts.next().unwrap());
	assert!(
		allocations > 0,
		"the first escaped argument is decoded on demand"
	);
	assert!(matches!(first.0, TextPlace::ToolCall { string: 1, .. }));
	assert_eq!(first.1.read(), "a\nb");
	let (second, allocations) = counted(|| texts.next().unwrap());
	assert!(
		allocations > 0,
		"the second escaped argument is decoded on demand"
	);
	assert!(matches!(second.0, TextPlace::ToolCall { string: 2, .. }));
	assert_eq!(second.1.read(), "c\nd");

	let (result, allocations) = counted(|| texts.next().unwrap());
	assert_eq!(allocations, 0);
	assert_eq!(result.1.read(), "result");
	assert!(texts.next().is_none());
}

fn counted<T>(read: impl FnOnce() -> T) -> (T, usize) {
	ALLOCATIONS.with(|count| count.set(Some(0)));
	let result = read();
	let count = ALLOCATIONS.with(|count| count.replace(None).unwrap());
	(result, count)
}

struct CountAllocations;

// SAFETY: Allocation and deallocation are forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for CountAllocations {
	unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
		let _ = ALLOCATIONS.try_with(|count| {
			if let Some(value) = count.get() {
				count.set(Some(value + 1));
			}
		});
		unsafe { System.alloc(layout) }
	}

	unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
		unsafe { System.dealloc(ptr, layout) }
	}
}

struct NoReplay;

impl IdentifierPseudonymizer for NoReplay {
	fn pseudonymize(&self, _: &str) -> String {
		unreachable!("this request does not send a safety identifier")
	}
}
