//! Incremental SSE decoding for provider streams.
//!
//! Events don't depend on how the input is split into reads, and LF, CR and CRLF line endings
//! are all accepted. Only each event's data is kept.
//!
//! An event over the limit is reported once as [`SseFrame::TooLarge`] and skipped, and decoding
//! resumes with the next event. The limit counts an event's line bytes, excluding line endings.

use std::io;
use std::ops::Range;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use memchr::memchr2;
use tokio_util::codec::Decoder;

const BOM: &[u8] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseFrame {
	/// The event's data lines, joined with `\n`.
	Event(Bytes),
	TooLarge,
}

#[derive(Debug, thiserror::Error)]
pub enum SseError {
	#[error("input ended inside an SSE event")]
	Truncated,
	#[error("SSE event over the size limit")]
	TooLarge,
	#[error(transparent)]
	Io(#[from] io::Error),
}

/// Keeps the current event's lines in the source buffer until the event ends, so single-line
/// data is returned without a copy.
pub struct SseDecoder {
	max_event_bytes: usize,
	/// Bytes of the source already read as lines of the current event.
	read: usize,
	/// Bytes after `read` already searched for a line ending.
	searched: usize,
	event_bytes: usize,
	data: Data,
	/// A non-comment line of the current event was read, so input ending now cuts the event.
	field_pending: bool,
	/// The last line ended in CR, so a following LF belongs to that ending.
	after_cr: bool,
	at_start: bool,
	/// Dropping an oversized event's lines until the blank line that ends it.
	skipping: bool,
	/// Part of the current line was already dropped, so its ending isn't a blank line.
	skipped_part_of_line: bool,
}

enum Data {
	None,
	/// A single data value, as a range of the source buffer.
	Line(Range<usize>),
	Joined(BytesMut),
}

impl SseDecoder {
	#[must_use]
	pub const fn new(max_event_bytes: usize) -> Self {
		Self {
			max_event_bytes,
			read: 0,
			searched: 0,
			event_bytes: 0,
			data: Data::None,
			field_pending: false,
			after_cr: false,
			at_start: true,
			skipping: false,
			skipped_part_of_line: false,
		}
	}
}

impl Decoder for SseDecoder {
	type Item = SseFrame;
	type Error = SseError;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<SseFrame>, SseError> {
		if self.at_start && !self.skip_bom(src) {
			return Ok(None);
		}
		loop {
			if self.after_cr {
				match src.get(self.read) {
					None => return Ok(None),
					Some(b'\n') => self.read += 1,
					Some(_) => {},
				}
				self.after_cr = false;
				continue;
			}
			let rest = &src[self.read..];
			let Some(found) = memchr2(b'\n', b'\r', &rest[self.searched..]) else {
				self.searched = rest.len();
				return Ok(self.unfinished_line(src));
			};
			let start = self.read;
			let end = start + self.searched + found;
			self.searched = 0;
			self.after_cr = src[end] == b'\r';
			self.read = end + 1;
			if let Some(frame) = self.line(src, start, end) {
				return Ok(Some(frame));
			}
		}
	}

	/// Some upstreams end with a `data: [DONE]` and no blank line; that event is still returned.
	/// Any other unfinished event is [`SseError::Truncated`].
	fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<SseFrame>, SseError> {
		if let Some(frame) = self.decode(src)? {
			return Ok(Some(frame));
		}
		let partial_bom = self.at_start && !src.is_empty();
		let partial_line = self.read < src.len();
		if !(self.skipping || partial_bom || partial_line || self.field_pending) {
			return Ok(None);
		}
		if self.skipping || partial_bom {
			return Err(SseError::Truncated);
		}
		if partial_line {
			let (start, end) = (self.read, src.len());
			self.read = end;
			self.searched = 0;
			let frame = self.line(src, start, end);
			assert!(
				frame.is_none(),
				"a non-blank line within the limit yields no frame"
			);
		}
		// Joined data contains a newline, so it can't be `[DONE]`.
		if let Data::Line(range) = &self.data
			&& src[range.clone()] == *b"[DONE]"
		{
			return Ok(self.dispatch(src));
		}
		Err(SseError::Truncated)
	}
}

impl SseDecoder {
	/// Reads the line at `start..end` of `src`, without its ending.
	fn line(&mut self, src: &mut BytesMut, start: usize, end: usize) -> Option<SseFrame> {
		if self.skipping {
			let blank = start == end && !self.skipped_part_of_line;
			self.skipped_part_of_line = false;
			self.skipping = !blank;
			src.advance(self.read);
			self.read = 0;
			return None;
		}
		if start == end {
			return self.dispatch(src);
		}
		self.event_bytes += end - start;
		if self.event_bytes > self.max_event_bytes {
			return Some(self.too_large(src));
		}
		let line = &src[start..end];
		if line[0] == b':' {
			return None;
		}
		self.field_pending = true;
		// Other fields are ignored. Per the SSE spec, a bare `data` is empty and one space after
		// the colon is dropped.
		let value = if line == b"data" {
			end
		} else {
			let value = line.strip_prefix(b"data:")?;
			end - value.strip_prefix(b" ").unwrap_or(value).len()
		};
		self.push_data(src, value..end);
		None
	}

	fn push_data(&mut self, src: &BytesMut, value: Range<usize>) {
		match &mut self.data {
			Data::None => self.data = Data::Line(value),
			Data::Line(first) => {
				let first = &src[first.clone()];
				let mut joined = BytesMut::with_capacity(first.len() + 1 + value.len());
				joined.extend_from_slice(first);
				joined.put_u8(b'\n');
				joined.extend_from_slice(&src[value]);
				self.data = Data::Joined(joined);
			},
			Data::Joined(joined) => {
				joined.put_u8(b'\n');
				joined.extend_from_slice(&src[value]);
			},
		}
	}

	/// Ends the current event, consuming its lines from `src`.
	fn dispatch(&mut self, src: &mut BytesMut) -> Option<SseFrame> {
		self.event_bytes = 0;
		self.field_pending = false;
		let data = std::mem::replace(&mut self.data, Data::None);
		let read = std::mem::take(&mut self.read);
		match data {
			Data::None => {
				src.advance(read);
				None
			},
			Data::Line(range) => Some(SseFrame::Event(src.split_to(read).freeze().slice(range))),
			Data::Joined(joined) => {
				src.advance(read);
				Some(SseFrame::Event(joined.freeze()))
			},
		}
	}

	/// A partial line that is being skipped or over the limit is dropped rather than held.
	fn unfinished_line(&mut self, src: &mut BytesMut) -> Option<SseFrame> {
		let partial = src.len() - self.read;
		if self.skipping {
			self.skipped_part_of_line |= partial > 0;
			src.clear();
			self.read = 0;
			self.searched = 0;
			return None;
		}
		if self.event_bytes + partial <= self.max_event_bytes {
			return None;
		}
		let frame = self.too_large(src);
		src.clear();
		self.searched = 0;
		self.skipped_part_of_line = true;
		Some(frame)
	}

	fn too_large(&mut self, src: &mut BytesMut) -> SseFrame {
		src.advance(self.read);
		self.read = 0;
		self.skipping = true;
		self.skipped_part_of_line = false;
		self.event_bytes = 0;
		self.data = Data::None;
		self.field_pending = false;
		SseFrame::TooLarge
	}

	/// Returns false until there are enough bytes to tell.
	fn skip_bom(&mut self, src: &mut BytesMut) -> bool {
		let available = src.len().min(BOM.len());
		if src[..available] != BOM[..available] {
			self.at_start = false;
			return true;
		}
		if available < BOM.len() {
			return false;
		}
		self.at_start = false;
		src.advance(BOM.len());
		true
	}
}

#[cfg(test)]
#[path = "sse_decoder_tests.rs"]
mod tests;
