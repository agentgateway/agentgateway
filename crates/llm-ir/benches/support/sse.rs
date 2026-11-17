//! Framing only: both parsers receive the same provider bytes and transport chunks.
use agent_llm_ir::sse::{Frame, SseParser};
use bytes::{Bytes, BytesMut};
use tokio_sse_codec::{Frame as TokioFrame, SseDecoder};
use tokio_util::codec::Decoder;

use super::{Bencher, LIMIT, Protocol, Size, black_box, fmt, reply_text};

#[divan::bench(args = cases(), min_time = 0.1, max_time = 1)]
fn ir(b: Bencher, case: &Case) {
	let chunks = case.input();
	let mut expected = Vec::new();
	let mut actual = Vec::new();
	read_tokio(&chunks, |name, data| {
		expected.push((name.to_vec(), data.to_vec()))
	});
	read_ir(&chunks, |name, data| {
		actual.push((name.to_vec(), data.to_vec()))
	});
	assert!(!expected.is_empty());
	assert!(
		actual == expected,
		"{case}: IR {} events, Tokio {} events; first mismatch {:?}",
		actual.len(),
		expected.len(),
		actual.iter().zip(&expected).position(|(a, b)| a != b)
	);
	b.counter(divan::counter::BytesCount::new(
		chunks.iter().map(Vec::len).sum::<usize>(),
	))
	.bench_local(|| {
		read_ir(black_box(&chunks), |name, data| {
			black_box((name, data));
		})
	});
}

#[divan::bench(args = cases(), min_time = 0.1, max_time = 1)]
fn tokio(b: Bencher, case: &Case) {
	let chunks = case.input();
	b.counter(divan::counter::BytesCount::new(
		chunks.iter().map(Vec::len).sum::<usize>(),
	))
	.bench_local(|| {
		read_tokio(black_box(&chunks), |name, data| {
			black_box((name, data));
		})
	});
}

fn read_ir(chunks: &[Vec<u8>], mut emit: impl FnMut(&[u8], &[u8])) {
	let mut parser = SseParser::new(LIMIT);
	for chunk in chunks {
		parser.push(chunk);
		while let Some(frame) = parser.next_frame() {
			let Frame::Event(event) = frame else {
				unreachable!("these events fit within the shared limit");
			};
			emit(event.name.unwrap_or(b"message"), event.data);
		}
	}
	parser.finish().unwrap();
}

fn read_tokio(chunks: &[Vec<u8>], mut emit: impl FnMut(&[u8], &[u8])) {
	let mut decoder = SseDecoder::<Bytes>::with_max_size(LIMIT);
	let mut buffer = BytesMut::new();
	for chunk in chunks {
		buffer.extend_from_slice(chunk);
		while let Some(frame) = decoder.decode(&mut buffer).unwrap() {
			if let TokioFrame::Event(event) = frame {
				emit(event.name.as_bytes(), &event.data);
			}
		}
	}
	assert!(decoder.decode_eof(&mut buffer).unwrap().is_none());
}

struct Case {
	protocol: Protocol,
	size: Size,
	chunk: usize,
}

impl fmt::Display for Case {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}/{}/", self.protocol.name(), self.size)?;
		match self.chunk {
			0 => f.write_str("events"),
			chunk => write!(f, "{chunk}B-reads"),
		}
	}
}

impl Case {
	fn input(&self) -> Vec<Vec<u8>> {
		let events = self.protocol.stream(&reply_text(self.size.0));
		if self.chunk == 0 {
			return events;
		}
		events
			.concat()
			.chunks(self.chunk)
			.map(<[u8]>::to_vec)
			.collect()
	}
}

fn cases() -> Vec<Case> {
	super::workload::PROTOCOLS
		.iter()
		.flat_map(|&protocol| {
			[Size(16 << 10), Size(128 << 10)]
				.into_iter()
				.flat_map(move |size| {
					[0, 7, 4096].map(move |chunk| Case {
						protocol,
						size,
						chunk,
					})
				})
		})
		// tokio_sse_codec 0.0.2 merges events when a read splits Gemini's CRLF.
		.filter(|case| case.protocol != Protocol::Gemini || case.chunk == 0)
		.collect()
}
