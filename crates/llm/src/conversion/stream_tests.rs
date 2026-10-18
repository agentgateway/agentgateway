//! Stream endings across client formats

use std::time::Duration;

use agent_http::Body;
use http_body_util::BodyExt;

use crate::{LogContentFields, StreamingUsageGuard};

#[derive(Clone, Copy)]
pub enum End {
	Close,
	Reset,
	Hang,
}

pub fn upstream(frames: &[&'static str], end: End) -> Body {
	use futures_util::StreamExt as _;
	let frames = frames
		.iter()
		.map(|f| Ok::<_, std::io::Error>(bytes::Bytes::from_static(f.as_bytes())))
		.chain(matches!(end, End::Reset).then(|| Err(std::io::Error::other("connection reset"))));
	let frames = futures_util::stream::iter(frames.collect::<Vec<_>>());
	match end {
		End::Hang => Body::from_stream(frames.chain(futures_util::stream::pending())),
		End::Close | End::Reset => Body::from_stream(frames),
	}
}

pub async fn drain(mut body: Body) -> (String, bool) {
	let mut text = String::new();
	while let Some(frame) = body.frame().await {
		match frame {
			Ok(frame) => text.push_str(std::str::from_utf8(frame.data_ref().unwrap()).unwrap()),
			Err(_) => return (text, true),
		}
	}
	(text, false)
}

pub fn last_event(text: &str) -> &str {
	text.trim_end().rsplit("\n\n").next().unwrap()
}

const CHUNK: &str = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\n";
const ERROR: &str =
	"data: {\"error\":{\"type\":\"server_error\",\"message\":\"The model is overloaded.\"}}\n\n";
const AFTER: &str = "data: {\"id\":\"c1\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"after\"},\"finish_reason\":\"stop\"}]}\n\n";
const DONE: &str = "data: [DONE]\n\n";

fn messages(b: Body) -> Body {
	super::completions::from_messages::translate_stream(
		b,
		1 << 20,
		StreamingUsageGuard::default(),
		LogContentFields::default(),
	)
}

fn responses(b: Body) -> Body {
	super::openai_compat::to_responses::translate_stream(
		b,
		1 << 20,
		StreamingUsageGuard::default(),
		LogContentFields::default(),
		None,
	)
}

/// Stop after the error event even if upstream sends more content, hangs or resets
#[rstest::rstest]
#[case::then_more_content(&[CHUNK, ERROR, AFTER, DONE], End::Close)]
#[case::then_an_open_stream(&[CHUNK, ERROR], End::Hang)]
#[case::then_a_reset(&[CHUNK, ERROR], End::Reset)]
#[tokio::test]
async fn an_upstream_error_event_ends_the_stream(
	#[values(messages, responses)] convert: fn(Body) -> Body,
	#[case] frames: &[&'static str],
	#[case] end: End,
) {
	let (text, failed) = tokio::time::timeout(
		Duration::from_secs(5),
		drain(convert(upstream(frames, end))),
	)
	.await
	.expect("the stream should end at the error event");
	assert!(
		!failed,
		"the body should end after the error event:\n{text}"
	);
	assert!(
		last_event(&text).contains("The model is overloaded."),
		"the error should be the last event:\n{text}"
	);
	assert!(!text.contains("after"), "content after the error:\n{text}");
}
