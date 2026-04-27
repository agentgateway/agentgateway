pub mod aws_sse;
pub mod passthrough;
pub mod sse;
pub mod transform;
pub mod websocket;

use bytes::{Bytes, BytesMut};

pub(crate) fn encode_sse_event(event_name: &str, data: Bytes) -> Bytes {
	let mut out = BytesMut::new();
	if !event_name.is_empty() {
		out.extend_from_slice(b"event: ");
		out.extend_from_slice(event_name.as_bytes());
		out.extend_from_slice(b"\n");
	}

	for line in data.split(|byte| *byte == b'\n') {
		out.extend_from_slice(b"data: ");
		out.extend_from_slice(line);
		out.extend_from_slice(b"\n");
	}
	out.extend_from_slice(b"\n");
	out.freeze()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
