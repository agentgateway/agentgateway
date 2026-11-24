use std::path::{Path, PathBuf};

use bytes::{Bytes, BytesMut};
use rstest::rstest;
use tokio_util::codec::Decoder;

use super::{SseDecoder, SseError, SseFrame};

/// Every frame from `chunks`, then the end-of-stream result.
fn decode(chunks: &[&[u8]], limit: usize) -> (Vec<SseFrame>, Result<(), String>) {
	let mut decoder = SseDecoder::new(limit);
	let mut buf = BytesMut::new();
	let mut frames = Vec::new();
	for chunk in chunks {
		buf.extend_from_slice(chunk);
		while let Some(frame) = decoder.decode(&mut buf).unwrap() {
			frames.push(frame);
		}
	}
	loop {
		match decoder.decode_eof(&mut buf) {
			Ok(Some(frame)) => frames.push(frame),
			Ok(None) => return (frames, Ok(())),
			Err(e) => return (frames, Err(e.to_string())),
		}
	}
}

fn event(data: &str) -> SseFrame {
	SseFrame::Event(Bytes::copy_from_slice(data.as_bytes()))
}

fn events(data: &[&str]) -> Vec<SseFrame> {
	data.iter().map(|data| event(data)).collect()
}

fn bytes<'a>(chunks: &[&'a str]) -> Vec<&'a [u8]> {
	chunks.iter().map(|chunk| chunk.as_bytes()).collect()
}

#[rstest]
#[case::crlf_split_between_reads(&["data: a\r\n\r", "\ndata: b\r\n\r\n"], &["a", "b"])]
#[case::bare_cr(&["data: a\r\rdata: b\r\r"], &["a", "b"])]
#[case::joins_data_and_ignores_other_lines(
	&[": note\nid: 1\nevent: e\ndata: a\ndata2: c\ndata:b\nretry: 3\n\n"],
	&["a\nb"]
)]
#[case::name_without_data(&["event: lost\n\ndata: x\n\n"], &["x"])]
#[case::empty_data(&["data:\n\n"], &[""])]
#[case::no_colon(&["data\n\n"], &[""])]
#[case::one_leading_space_stripped(&["data:  x \n\n"], &[" x "])]
#[case::ends_after_comment(&["data: x\n\n: ping\n"], &["x"])]
#[case::unterminated_done(&["data: x\n\n", "data: [DONE]"], &["x", "[DONE]"])]
#[case::unterminated_done_lf(&["data: x\n\n", "data: [DONE]\n"], &["x", "[DONE]"])]
#[case::unterminated_done_cr(&["data: x\n\n", "data: [DONE]\r"], &["x", "[DONE]"])]
#[case::unterminated_done_crlf(&["data: x\n\n", "data: [DONE]\r\n"], &["x", "[DONE]"])]
fn decodes(#[case] chunks: &[&str], #[case] expected: &[&str]) {
	let (frames, end) = decode(&bytes(chunks), 1024);
	assert_eq!(frames, events(expected));
	end.unwrap();
}

#[rstest]
#[case::cut_event(&["data: x\n"], &[])]
#[case::not_sse_after_an_event(&["data: {}\n\nnot sse"], &["{}"])]
#[case::cut_after_event_name(&["data: x\n\n", "event: message_start\n"], &["x"])]
#[case::cut_after_id(&["data: x\n\n", "id: 1\r\n"], &["x"])]
#[case::cut_after_retry(&["data: x\n\n", "retry: 10\r"], &["x"])]
#[case::done_then_more_data(&["data: [DONE]\ndata: more"], &[])]
fn truncated(#[case] chunks: &[&str], #[case] expected: &[&str]) {
	let (frames, end) = decode(&bytes(chunks), 1024);
	assert_eq!(frames, events(expected));
	assert_eq!(end, Err(SseError::Truncated.to_string()));
}

#[test]
fn a_bom_split_between_reads_is_skipped() {
	let (frames, end) = decode(&[b"\xEF", b"\xBB\xBFdata: x\n\n"], 1024);
	assert_eq!(frames, [event("x")]);
	end.unwrap();
}

#[test]
fn an_event_over_the_limit_is_reported_once() {
	let (frames, _) = decode(&[b"data: 0123456789\n\n"], 8);
	assert_eq!(frames, [SseFrame::TooLarge]);
	let split: &[&[u8]] = &[b"da", b"ta", b": 01234567", b"89", b"\n", b"\n"];
	let (frames, _) = decode(split, 8);
	assert_eq!(frames, [SseFrame::TooLarge]);
}

#[test]
fn events_after_a_large_one_stay_whole() {
	let (frames, end) = decode(&[b"data: 0123456789\n\ndata: x\n\n"], 10);
	assert_eq!(frames, [SseFrame::TooLarge, event("x")]);
	end.unwrap();
}

#[test]
fn a_large_cut_event_is_truncated() {
	let (frames, end) = decode(&[b"data: 0123456789"], 8);
	assert_eq!(frames, [SseFrame::TooLarge]);
	assert_eq!(end, Err(SseError::Truncated.to_string()));
}

#[test]
fn a_large_event_holds_no_more_than_a_chunk() {
	let mut decoder = SseDecoder::new(8);
	let mut buf = BytesMut::from(&b"data: "[..]);
	for _ in 0..100 {
		buf.extend_from_slice(b"0123456789");
		while decoder.decode(&mut buf).unwrap().is_some() {}
		assert!(buf.is_empty());
	}
}

#[test]
fn comments_over_the_limit_are_too_large() {
	let (frames, _) = decode(&[b": a very long comment\n\ndata: x\n\n"], 10);
	assert_eq!(frames, [SseFrame::TooLarge, event("x")]);
}

#[test]
fn a_single_data_line_is_not_copied() {
	let mut decoder = SseDecoder::new(1024);
	let mut buf = BytesMut::from(&b"data: x\n\n"[..]);
	let start = buf.as_ptr();
	let Some(SseFrame::Event(data)) = decoder.decode(&mut buf).unwrap() else {
		panic!("expected an event");
	};
	assert_eq!(data.as_ptr(), start.wrapping_add("data: ".len()));
}

/// Every provider stream fixture, with its own line endings and rewritten to CRLF and CR.
fn corpus() -> Vec<(String, Vec<u8>)> {
	let mut files = Vec::new();
	collect_files(Path::new("src/tests/response"), &mut files);
	let mut corpus = Vec::new();
	for path in files {
		let input = std::fs::read(&path).unwrap();
		if path.extension().is_some_and(|ext| ext == "snap")
			|| !input.starts_with(b"data:") && !input.starts_with(b"event:")
		{
			continue;
		}
		let lf = String::from_utf8(input.clone())
			.unwrap()
			.replace("\r\n", "\n");
		let name = path.display().to_string();
		corpus.push((
			format!("{name} (CRLF)"),
			lf.replace('\n', "\r\n").into_bytes(),
		));
		corpus.push((format!("{name} (CR)"), lf.replace('\n', "\r").into_bytes()));
		corpus.push((name, input));
	}
	assert!(corpus.len() > 60, "found only {} fixtures", corpus.len());
	corpus
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in std::fs::read_dir(dir).unwrap() {
		let path = entry.unwrap().path();
		if path.is_dir() {
			collect_files(&path, out);
		} else {
			out.push(path);
		}
	}
}

#[test]
fn provider_streams_decode_the_same_however_they_are_split() {
	for (name, input) in corpus() {
		// The small limit also covers recovery from oversized events.
		for limit in [usize::MAX, 200] {
			let whole = decode(&[&input], limit);
			if limit == usize::MAX {
				assert!(!whole.0.is_empty(), "{name}: no events");
			}
			let bytes: Vec<&[u8]> = input.chunks(1).collect();
			assert_eq!(
				decode(&bytes, limit),
				whole,
				"{name}: 1-byte reads, limit {limit}"
			);
			for split in 1..input.len() {
				let (a, b) = input.split_at(split);
				assert_eq!(
					decode(&[a, b], limit),
					whole,
					"{name}: split at {split}, limit {limit}"
				);
			}
		}
	}
}
