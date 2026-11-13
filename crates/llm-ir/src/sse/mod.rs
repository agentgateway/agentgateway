// TODO: we should move away from tokio_sse_codec see benchmark

//! Incremental SSE framing for provider replies and framing helpers for client output.
//!
//! Complete events borrow the parser's buffer. Multiple data lines are joined;
//! a single data line needs no extra copy. Transport chunk boundaries do not
//! change the parsed events.
//!
//! Oversized events are returned in pieces instead of retained whole across
//! chunks. The event limit does not bound total memory: `push` copies the entire
//! input chunk, and buffers retain their allocated capacity.

use memchr::memchr2;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// One complete SSE event borrowed from the parser.
#[derive(Debug, PartialEq, Eq)]
pub struct Event<'a> {
	pub name: Option<&'a [u8]>,
	pub data: &'a [u8],
}

/// A complete SSE event or one piece of an oversized event.
#[derive(Debug, PartialEq, Eq)]
pub enum Frame<'a> {
	Event(Event<'a>),
	/// The next bytes of an event over the limit.
	Piece(&'a [u8]),
	PiecesEnd,
}

/// Invalid SSE framing at the end of input.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SseError {
	#[error("input ended inside an SSE event")]
	Truncated,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LargeLine {
	Start,
	Data,
	Skipped,
}

#[derive(Clone, Copy)]
struct Span {
	start: usize,
	end: usize,
}

/// Incrementally frames SSE bytes without performing I/O.
// Cursors satisfy `event_start <= line_start <= scanned <= buffer.len()`.
// `data` borrows the first data line until a second line moves it into `joined`;
// `is_joined` and `data.is_some()` are mutually exclusive.
pub struct SseParser {
	buffer: Vec<u8>,
	max_event_bytes: usize,
	/// Bytes before it are finished with.
	event_start: usize,
	line_start: usize,
	/// How far line endings have been searched, so nothing is scanned twice.
	scanned: usize,
	/// The last line ended in CR, so a following LF belongs to that ending.
	skip_lf: bool,
	at_start: bool,
	name: Option<Span>,
	data: Option<Span>,
	/// Joined text, used only for events with several data lines.
	joined: Vec<u8>,
	is_joined: bool,
	large_event: Option<LargeLine>,
	sent_a_piece: bool,
}

impl SseParser {
	#[must_use]
	pub const fn new(max_event_bytes: usize) -> Self {
		Self {
			buffer: Vec::new(),
			max_event_bytes,
			event_start: 0,
			line_start: 0,
			scanned: 0,
			skip_lf: false,
			at_start: true,
			name: None,
			data: None,
			joined: Vec::new(),
			is_joined: false,
			large_event: None,
			sent_a_piece: false,
		}
	}

	/// Adds input. Pull every event out before pushing more, or input kept here
	/// grows by the size of each chunk.
	pub fn push(&mut self, chunk: &[u8]) {
		self.debug_assert_state();
		self.drop_finished_bytes();
		self.buffer.extend_from_slice(chunk);
	}

	/// The next frame, or `None` when more input is needed.
	pub fn next_frame(&mut self) -> Option<Frame<'_>> {
		self.debug_assert_state();
		if self.at_start && !self.skip_bom() {
			return None;
		}
		if self.large_event.is_some() {
			return self.next_large_frame();
		}
		loop {
			if !self.pass_lf_after_cr() {
				return None;
			}
			let rest = &self.buffer[self.scanned..];
			let Some(offset) = memchr2(b'\n', b'\r', rest) else {
				self.scanned = self.buffer.len();
				if !self.is_over_limit() {
					return None;
				}
				self.scanned = self.line_start;
				return self.start_large_event();
			};
			let line_end = self.scanned + offset;
			let line_start = self.line_start;
			self.scanned = line_end + 1;
			let over_limit = self.is_over_limit();
			if line_end == line_start {
				if over_limit {
					self.scanned = line_end;
					return self.start_large_event();
				}
				self.skip_lf = self.buffer[line_end] == b'\r';
				self.line_start = self.scanned;
				if self.data.is_some() || self.is_joined {
					return Some(Frame::Event(self.take_event()));
				}
				self.name = None;
				self.event_start = self.scanned;
				continue;
			}
			self.skip_lf = self.buffer[line_end] == b'\r';
			self.line_start = self.scanned;
			self.read_line(line_start, line_end);
			if over_limit {
				return self.start_large_event();
			}
		}
	}

	/// Checks for an incomplete event at end of input. Drain `next_frame` first.
	pub fn finish(&self) -> Result<(), SseError> {
		self.debug_assert_state();
		let unfinished = self.large_event.is_some()
			|| self.line_start < self.buffer.len()
			|| self.data.is_some()
			|| self.is_joined;
		if unfinished {
			Err(SseError::Truncated)
		} else {
			Ok(())
		}
	}

	const fn is_over_limit(&self) -> bool {
		self.scanned - self.event_start > self.max_event_bytes
	}

	/// Returns false when the byte after a CR hasn't arrived yet.
	fn pass_lf_after_cr(&mut self) -> bool {
		if self.skip_lf {
			match self.buffer.get(self.scanned) {
				None => return false,
				Some(b'\n') => self.scanned += 1,
				Some(_) => {},
			}
			self.skip_lf = false;
			self.line_start = self.scanned;
		}
		true
	}

	fn start_large_event(&mut self) -> Option<Frame<'_>> {
		self.large_event = Some(LargeLine::Start);
		self.name = None;
		self.event_start = self.line_start;
		let data = self.data.take();
		if std::mem::take(&mut self.is_joined) {
			self.sent_a_piece = true;
			return Some(Frame::Piece(&self.joined));
		}
		let Some(span) = data else {
			return self.next_large_frame();
		};
		// The event has data, even if this line's is empty, as when the line is read in pieces.
		self.sent_a_piece = true;
		if span.start == span.end {
			return self.next_large_frame();
		}
		Some(Frame::Piece(&self.buffer[span.start..span.end]))
	}

	fn next_large_frame(&mut self) -> Option<Frame<'_>> {
		loop {
			if !self.pass_lf_after_cr() {
				return None;
			}
			let rest = &self.buffer[self.scanned..];
			let line_end = memchr2(b'\n', b'\r', rest);
			if self.large_event == Some(LargeLine::Start) {
				if line_end == Some(0) {
					self.end_line(0);
					self.large_event = None;
					self.event_start = self.scanned;
					if std::mem::take(&mut self.sent_a_piece) {
						return Some(Frame::PiecesEnd);
					}
					return self.next_frame();
				}
				// Enough to tell the field, and any space after its colon.
				if line_end.is_none() && rest.len() < 6 {
					return None;
				}
				let line = &rest[..line_end.unwrap_or(rest.len())];
				let value_start = match line.strip_prefix(b"data:") {
					Some([b' ', ..]) => Some(6),
					Some(_) => Some(5),
					None => (line == b"data").then_some(4),
				};
				let Some(value_start) = value_start else {
					self.large_event = Some(LargeLine::Skipped);
					continue;
				};
				self.scanned += value_start;
				self.large_event = Some(LargeLine::Data);
				self.event_start = self.scanned;
				self.line_start = self.scanned;
				if std::mem::replace(&mut self.sent_a_piece, true) {
					return Some(Frame::Piece(b"\n"));
				}
				continue;
			}
			let start = self.scanned;
			let length = line_end.unwrap_or(rest.len());
			let is_data = self.large_event == Some(LargeLine::Data);
			match line_end {
				Some(length) => {
					self.end_line(length);
					self.large_event = Some(LargeLine::Start);
				},
				None => {
					self.scanned += length;
					self.line_start = self.scanned;
				},
			}
			self.event_start = self.scanned;
			if is_data && length > 0 {
				return Some(Frame::Piece(&self.buffer[start..start + length]));
			}
			line_end?;
		}
	}

	fn end_line(&mut self, length: usize) {
		self.scanned += length;
		self.skip_lf = self.buffer[self.scanned] == b'\r';
		self.scanned += 1;
		self.line_start = self.scanned;
	}

	/// Returns false until there are enough bytes to tell.
	fn skip_bom(&mut self) -> bool {
		let available = self.buffer.len().min(BOM.len());
		if self.buffer[..available] != BOM[..available] {
			self.at_start = false;
			return true;
		}
		if available < BOM.len() {
			return false;
		}
		self.at_start = false;
		self.event_start = BOM.len();
		self.line_start = BOM.len();
		self.scanned = BOM.len();
		true
	}

	fn read_line(&mut self, start: usize, end: usize) {
		debug_assert!(start < end);
		let line = &self.buffer[start..end];
		if line[0] == b':' {
			return;
		}
		let (field, value_start) = split_field(line);
		let value = Span {
			start: start + value_start,
			end,
		};
		match field {
			b"event" => self.name = Some(value),
			b"data" => self.add_data(value),
			_ => {},
		}
	}

	fn add_data(&mut self, value: Span) {
		if !self.is_joined {
			let Some(first) = self.data.take() else {
				self.data = Some(value);
				return;
			};
			self.joined.clear();
			self
				.joined
				.extend_from_slice(&self.buffer[first.start..first.end]);
			self.is_joined = true;
		}
		self.joined.push(b'\n');
		self
			.joined
			.extend_from_slice(&self.buffer[value.start..value.end]);
	}

	fn take_event(&mut self) -> Event<'_> {
		let name = self.name.take();
		let data = self.data.take();
		let is_joined = std::mem::take(&mut self.is_joined);
		self.event_start = self.scanned;
		let data = match data {
			Some(span) if !is_joined => &self.buffer[span.start..span.end],
			_ => &self.joined[..],
		};
		Event {
			name: name.map(|span| &self.buffer[span.start..span.end]),
			data,
		}
	}

	fn drop_finished_bytes(&mut self) {
		let done = self.event_start;
		if done == 0 {
			return;
		}
		self.buffer.drain(..done);
		self.event_start = 0;
		self.line_start -= done;
		self.scanned -= done;
		for span in [&mut self.name, &mut self.data].into_iter().flatten() {
			span.start -= done;
			span.end -= done;
		}
	}

	fn debug_assert_state(&self) {
		debug_assert!(self.event_start <= self.line_start);
		debug_assert!(self.line_start <= self.scanned);
		debug_assert!(self.scanned <= self.buffer.len());
		debug_assert!(!self.is_joined || self.data.is_none());
		for span in [self.name, self.data].into_iter().flatten() {
			debug_assert!(self.event_start <= span.start);
			debug_assert!(span.start <= span.end);
			debug_assert!(span.end <= self.line_start);
		}
	}
}

fn split_field(line: &[u8]) -> (&[u8], usize) {
	let colon = line.iter().position(|byte| *byte == b':');
	let field = &line[..colon.unwrap_or(line.len())];
	let mut value_start = colon.map_or(line.len(), |colon| colon + 1);
	if line.get(value_start) == Some(&b' ') {
		value_start += 1;
	}
	(field, value_start)
}

/// Room for the framing and a typical small event.
const EVENT_ROOM: usize = 192;

/// The caller writes the data straight into `out`, so a large value isn't copied. It
/// must be one line, as JSON from these writers is.
pub(crate) fn open_event(out: &mut Vec<u8>, name: &str) {
	// One step, so writing the event in parts doesn't grow `out` several times.
	out.reserve(EVENT_ROOM + name.len());
	out.extend_from_slice(b"event: ");
	out.extend_from_slice(name.as_bytes());
	out.extend_from_slice(b"\ndata: ");
}

pub(crate) fn close_event(out: &mut Vec<u8>) {
	out.extend_from_slice(b"\n\n");
}

#[cfg(test)]
mod tests;
