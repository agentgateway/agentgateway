use agent_llm_ir::ir::events::PartId;
use agent_llm_ir::{ReplyPlace, ReplyRewrite};
use proptest::prelude::*;
use rstest::rstest;

#[path = "../benches/support/rewrite.rs"]
mod rewrite;

use rewrite::Replace;

#[rstest]
#[case(&["he b", "ody "], "he BODY ")]
#[case(&["b", "o", "d", "y"], "BODY")]
#[case(&["bodybod", "ybo"], "BODYBODYbo")]
#[case(&["bo", "日", "body", "é"], "bo日BODYé")]
fn replacement_survives_chunk_boundaries(#[case] chunks: &[&str], #[case] expected: &str) {
	let mut rewrite = Replace::new("body", "BODY");
	let place = ReplyPlace::Answer(PartId(0));
	let mut out = String::new();
	for chunk in chunks {
		rewrite.text(place, chunk, &mut out);
	}
	rewrite.finish(place, &mut out);
	assert_eq!(out, expected);
}

#[test]
fn partial_matches_belong_to_their_reply_place() {
	let mut rewrite = Replace::new("body", "BODY");
	let answer = ReplyPlace::Answer(PartId(0));
	let reasoning = ReplyPlace::Reasoning(PartId(1));
	let argument = ReplyPlace::ToolCallArgument {
		part: PartId(2),
		string: 0,
	};
	let next_argument = ReplyPlace::ToolCallArgument {
		part: PartId(2),
		string: 1,
	};
	let mut answer_out = String::new();
	let mut reasoning_out = String::new();
	let mut argument_out = String::new();
	let mut next_argument_out = String::new();
	rewrite.text(answer, "bo", &mut answer_out);
	rewrite.text(reasoning, "b", &mut reasoning_out);
	rewrite.text(argument, "bo", &mut argument_out);
	rewrite.finish(argument, &mut argument_out);
	rewrite.text(next_argument, "dy", &mut next_argument_out);
	rewrite.finish(next_argument, &mut next_argument_out);
	rewrite.text(answer, "dy", &mut answer_out);
	rewrite.finish(answer, &mut answer_out);
	rewrite.text(reasoning, "ody", &mut reasoning_out);
	rewrite.finish(reasoning, &mut reasoning_out);
	assert_eq!(answer_out, "BODY");
	assert_eq!(reasoning_out, "BODY");
	assert_eq!(argument_out, "bo");
	assert_eq!(next_argument_out, "dy");
}

proptest! {
	#[test]
	fn chunked_replacement_matches_whole_text(
		text in "(body|b|bo|bod|y|é|日|aba|x){0,40}",
		sizes in prop::collection::vec(1usize..9, 1..20),
		from in prop::sample::select(vec!["body", "é日", "aba"]),
	) {
		let mut rewrite = Replace::new(from, "replacement");
		let place = ReplyPlace::Answer(PartId(0));
		let mut out = String::new();
		let chars: Vec<char> = text.chars().collect();
		let mut at = 0;
		for size in sizes.iter().cycle() {
			if at == chars.len() { break; }
			let end = (at + size).min(chars.len());
			let chunk: String = chars[at..end].iter().collect();
			rewrite.text(place, &chunk, &mut out);
			at = end;
		}
		rewrite.finish(place, &mut out);
		prop_assert_eq!(out, text.replace(from, "replacement"));
	}
}
