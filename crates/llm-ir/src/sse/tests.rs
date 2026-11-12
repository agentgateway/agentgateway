#![allow(clippy::panic, clippy::unwrap_used)]

use proptest::prelude::*;
use rstest::rstest;

use super::*;

fn all(parser: &mut SseParser) -> Vec<(Option<Vec<u8>>, Vec<u8>)> {
	let mut out = Vec::new();
	while let Some(frame) = parser.next_frame() {
		let Frame::Event(event) = frame else {
			unreachable!("{frame:?}");
		};
		out.push((event.name.map(<[u8]>::to_vec), event.data.to_vec()));
	}
	out
}

fn one_chunk(input: &[u8]) -> Vec<(Option<Vec<u8>>, Vec<u8>)> {
	let mut parser = SseParser::new(1024);
	parser.push(input);
	let out = all(&mut parser);
	parser.finish().unwrap();
	out
}

#[test]
fn crlf_split_between_cr_and_lf_is_one_line_ending() {
	let mut parser = SseParser::new(1024);
	parser.push(b"data: x\r");
	assert_eq!(all(&mut parser), []);
	parser.push(b"\n\r");
	assert_eq!(all(&mut parser), [(None, b"x".to_vec())]);
	parser.push(b"\n");
	assert_eq!(all(&mut parser), []);
	parser.finish().unwrap();
}

#[test]
fn joins_data_lines_and_skips_comments_and_other_fields() {
	let got = one_chunk(b": note\nid: 1\nevent: e\ndata: a\ndata:b\nretry: 3\n\n");
	assert_eq!(got, [(Some(b"e".to_vec()), b"a\nb".to_vec())]);
}

#[test]
fn a_name_without_data_is_no_event() {
	assert_eq!(
		one_chunk(b"event: lost\n\ndata: x\n\n"),
		[(None, b"x".to_vec())]
	);
}

#[test]
fn empty_data_is_an_event() {
	assert_eq!(one_chunk(b"data:\n\n"), [(None, b"".to_vec())]);
}

#[test]
fn a_field_with_no_colon_has_an_empty_value() {
	assert_eq!(one_chunk(b"data\n\n"), [(None, b"".to_vec())]);
}

#[test]
fn a_leading_bom_is_skipped_even_when_split() {
	let mut parser = SseParser::new(1024);
	parser.push(b"\xEF");
	assert_eq!(all(&mut parser), []);
	parser.push(b"\xBB\xBFdata: x\n\n");
	assert_eq!(all(&mut parser), [(None, b"x".to_vec())]);
}

#[test]
fn a_cut_event_is_truncated() {
	let mut parser = SseParser::new(1024);
	parser.push(b"data: x\n");
	assert_eq!(all(&mut parser), []);
	assert_eq!(parser.finish(), Err(SseError::Truncated));
}

fn pieces(chunks: &[&[u8]], limit: usize) -> (Vec<u8>, usize) {
	let mut parser = SseParser::new(limit);
	let mut data = Vec::new();
	let mut ends = 0;
	for chunk in chunks {
		parser.push(chunk);
		while let Some(frame) = parser.next_frame() {
			match frame {
				Frame::Piece(piece) => data.extend_from_slice(piece),
				Frame::PiecesEnd => ends += 1,
				Frame::Event(event) => panic!("whole event {event:?}"),
			}
		}
	}
	(data, ends)
}

#[test]
fn an_event_over_the_limit_arrives_in_pieces() {
	let whole: &[&[u8]] = &[b"data: 0123456789\n\n"];
	assert_eq!(pieces(whole, 8), (b"0123456789".to_vec(), 1));
	let split: &[&[u8]] = &[b"da", b"ta", b": 01234567", b"89", b"\n", b"\n"];
	assert_eq!(pieces(split, 8), (b"0123456789".to_vec(), 1));
}

#[test]
fn pieces_join_data_lines_and_skip_other_fields() {
	let got = pieces(
		&[b"event: e\ndata: 0123\n: note\nid: 1\ndata:45678\ndata\n\n"],
		8,
	);
	assert_eq!(got, (b"0123\n45678\n".to_vec(), 1));
}

/// Every frame, with each piece kept apart.
fn frames(chunks: &[&[u8]], limit: usize) -> Vec<String> {
	let mut parser = SseParser::new(limit);
	let mut frames = Vec::new();
	for chunk in chunks {
		parser.push(chunk);
		while let Some(frame) = parser.next_frame() {
			frames.push(format!("{frame:?}"));
		}
	}
	frames
}

#[rstest]
#[case::empty_event(b"data\n\n")]
#[case::empty_first_line(b"data\ndata: x\n\n")]
fn an_empty_data_line_over_the_limit_sends_no_piece(#[case] input: &[u8]) {
	let whole = frames(&[input], 1);
	let bytes: Vec<&[u8]> = input.chunks(1).collect();
	assert_eq!(whole, frames(&bytes, 1), "{input:?}");
	assert!(
		!whole.contains(&"Piece([])".to_string()),
		"{input:?}: {whole:?}"
	);
}

#[test]
fn events_after_a_large_one_stay_whole() {
	let mut parser = SseParser::new(10);
	parser.push(b"data: 0123456789\n\ndata: x\n\n");
	let mut frames = 0;
	while let Some(frame) = parser.next_frame() {
		frames += 1;
		if frames == 3 {
			assert_eq!(
				frame,
				Frame::Event(Event {
					name: None,
					data: b"x"
				})
			);
		}
	}
	assert_eq!(frames, 3);
	parser.finish().unwrap();
}

#[test]
fn a_large_cut_event_is_truncated() {
	let mut parser = SseParser::new(8);
	parser.push(b"data: 0123456789");
	while parser.next_frame().is_some() {}
	assert_eq!(parser.finish(), Err(SseError::Truncated));
}

#[test]
fn a_large_event_buffers_no_more_than_a_chunk() {
	let mut parser = SseParser::new(8);
	parser.push(b"data: ");
	for _ in 0..100 {
		parser.push(b"0123456789");
		while parser.next_frame().is_some() {}
	}
	parser.push(b"");
	assert_eq!(parser.buffer.len(), 0);
}

#[test]
fn comments_over_the_limit_are_no_event() {
	let mut parser = SseParser::new(10);
	parser.push(b": a very long comment\n\ndata: x\n\n");
	let frame = parser.next_frame();
	assert_eq!(
		frame,
		Some(Frame::Event(Event {
			name: None,
			data: b"x"
		}))
	);
}

#[test]
fn finished_bytes_are_released() {
	let mut parser = SseParser::new(1024);
	for _ in 0..100 {
		parser.push(b"data: 0123456789\n\n");
		assert_eq!(all(&mut parser).len(), 1);
	}
	parser.push(b"");
	assert_eq!(parser.buffer.len(), 0);
}

#[test]
fn a_chunk_with_many_events_gives_them_one_at_a_time() {
	let mut parser = SseParser::new(1024);
	parser.push(b"data: a\n\ndata: b\n\n");
	let data = |frame: Option<Frame<'_>>| match frame {
		Some(Frame::Event(event)) => event.data.to_vec(),
		other => panic!("{other:?}"),
	};
	assert_eq!(data(parser.next_frame()), b"a");
	assert_eq!(data(parser.next_frame()), b"b");
	assert_eq!(parser.next_frame(), None);
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(128))]

	#[test]
	fn generated_sse_payloads_survive_any_chunking(
		events in prop::collection::vec(prop::collection::vec(r"[^\r\n]{0,16}", 1..4), 0..8),
		ending in prop::sample::select(vec!["\n", "\r\n", "\r"]),
		bom in any::<bool>(), limit in 1..128usize,
		cuts in prop::collection::vec(any::<usize>(), 0..32),
	) {
		let mut input = if bom { "\u{feff}".to_string() } else { String::new() };
		let expected: Vec<Vec<u8>> = events.iter().map(|lines| lines.join("\n").into_bytes()).collect();
		for lines in &events {
			input.push_str(": comment");
			input.push_str(ending);
			for line in lines {
				input.push_str("data: ");
				input.push_str(line);
				input.push_str(ending);
			}
			input.push_str(ending);
		}
		let mut offsets: Vec<_> = cuts.iter().map(|cut| cut % (input.len() + 1)).collect();
		offsets.extend([0, input.len()]);
		offsets.sort_unstable();
		offsets.dedup();
		let mut parser = SseParser::new(limit);
		let mut actual = Vec::new();
		let mut pieces = Vec::new();
		for pair in offsets.windows(2) {
			parser.push(&input.as_bytes()[pair[0]..pair[1]]);
			while let Some(frame) = parser.next_frame() {
				match frame {
					Frame::Event(event) => {
						prop_assert!(event.data.len() <= limit);
						actual.push(event.data.to_vec());
					}
					Frame::Piece(piece) => pieces.extend_from_slice(piece),
					Frame::PiecesEnd => actual.push(std::mem::take(&mut pieces)),
				}
			}
		}
		prop_assert_eq!(parser.finish(), Ok(()));
		prop_assert!(pieces.is_empty());
		prop_assert_eq!(actual, expected);
	}
}

proptest! {
	#![proptest_config(ProptestConfig::with_cases(128))]

	#[test]
	fn arbitrary_sse_bytes_do_not_depend_on_chunk_size(
		input in prop::collection::vec(any::<u8>(), 0..1024),
		limit in prop_oneof![Just(usize::MAX), 1..256usize],
		chunk in 1..64usize,
	) {
		let whole = collected_frames(&input, limit, input.len().max(1));
		prop_assert_eq!(&whole, &collected_frames(&input, limit, chunk));
		prop_assert_eq!(&whole, &collected_frames(&input, limit, 1));
	}
}

#[derive(Debug, PartialEq)]
enum CollectedFrame {
	Event {
		name: Option<Vec<u8>>,
		data: Vec<u8>,
	},
	Pieces(Vec<u8>),
	PiecesEnd,
}

fn collected_frames(input: &[u8], limit: usize, chunk: usize) -> (Vec<CollectedFrame>, bool) {
	let mut parser = SseParser::new(limit);
	let mut out = Vec::new();
	for chunk in input.chunks(chunk) {
		parser.push(chunk);
		while let Some(frame) = parser.next_frame() {
			match frame {
				Frame::Event(event) => {
					assert!(event.data.len() <= limit);
					out.push(CollectedFrame::Event {
						name: event.name.map(<[u8]>::to_vec),
						data: event.data.to_vec(),
					});
				},
				Frame::Piece(piece) => match out.last_mut() {
					// Piece boundaries follow transport chunks, not event boundaries.
					Some(CollectedFrame::Pieces(joined)) => joined.extend_from_slice(piece),
					_ => out.push(CollectedFrame::Pieces(piece.to_vec())),
				},
				Frame::PiecesEnd => out.push(CollectedFrame::PiecesEnd),
			}
		}
	}
	(out, parser.finish().is_ok())
}
