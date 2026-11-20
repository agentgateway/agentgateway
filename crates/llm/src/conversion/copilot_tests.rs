use agent_http::Body;
use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_sse_codec::{Frame, SseDecoder};
use tokio_util::codec::Decoder;

use super::normalize_responses_stream;

const FIXTURE: &str = include_str!("../tests/response/copilot/unstable-item-ids.json");
const LIMIT: usize = 1024 * 1024;

fn events(bytes: &[u8]) -> Vec<Value> {
	let mut decoder = SseDecoder::<Bytes>::new();
	let mut bytes = BytesMut::from(bytes);
	let mut events = Vec::new();
	while let Some(frame) = decoder.decode_eof(&mut bytes).unwrap() {
		if let Frame::Event(event) = frame
			&& event.data.as_ref() != b"[DONE]"
		{
			events.push(serde_json::from_slice(&event.data).unwrap());
		}
	}
	events
}

fn event(value: Value) -> String {
	format!("data: {value}\n\n")
}

async fn normalize(input: impl Into<Body>) -> Bytes {
	normalize_responses_stream(input.into(), LIMIT)
		.collect()
		.await
		.unwrap()
		.to_bytes()
}

#[tokio::test]
async fn copilot_responses_stream_keeps_item_identity_and_payloads() {
	let original = events(FIXTURE.as_bytes());
	let output = normalize(FIXTURE).await;
	let output = events(&output);
	assert_eq!(output.len(), original.len());
	let anchors = ["rs_anchor", "msg_anchor", "fc_anchor"];
	for (mut expected, actual) in original.into_iter().zip(&output) {
		if let Some(index) = expected["output_index"].as_u64() {
			let id = json!(anchors[index as usize]);
			if expected.get("item_id").is_some() {
				expected["item_id"] = id;
			} else {
				expected["item"]["id"] = id;
			}
		}
		if let Some(items) = expected
			.get_mut("response")
			.and_then(|response| response.get_mut("output"))
			.and_then(Value::as_array_mut)
		{
			for (item, id) in items.iter_mut().zip(anchors) {
				item["id"] = json!(id);
			}
		}
		assert_eq!(&expected, actual);
	}
	let final_output = &output.last().unwrap()["response"]["output"];
	assert_eq!(final_output[0]["encrypted_content"], "opaque-reasoning");
	assert_eq!(final_output[2]["call_id"], "call_weather");
	assert_eq!(final_output[2]["arguments"], r#"{"location":"Shanghai"}"#);
}

#[tokio::test]
async fn copilot_responses_stream_isolates_concurrent_streams() {
	let first = normalize(FIXTURE);
	let other = FIXTURE
		.replace("rs_anchor", "rs_other")
		.replace("msg_anchor", "msg_other");
	let (first, second) = tokio::join!(first, normalize(other));
	assert_eq!(events(&first)[3]["item_id"], "rs_anchor");
	assert_eq!(events(&second)[3]["item_id"], "rs_other");
	assert_eq!(events(&first)[9]["item_id"], "msg_anchor");
	assert_eq!(events(&second)[9]["item_id"], "msg_other");
}

#[tokio::test]
async fn copilot_responses_stream_keeps_first_id_for_interleaved_items() {
	let input = [
		json!({"type":"response.output_item.added","output_index":0,"item":{"id":"first"}}),
		json!({"type":"response.output_item.added","output_index":1,"item":{"id":"second"}}),
		json!({"type":"response.output_text.delta","output_index":0,"item_id":"changed-first","delta":"a"}),
		json!({"type":"response.output_item.added","output_index":0,"item":{"id":"replacement"}}),
		json!({"type":"response.output_text.delta","output_index":1,"item_id":"changed-second","delta":"b"}),
		json!({"type":"response.output_item.done","output_index":0,"item":{"id":"done-first"}}),
		json!({"type":"response.output_item.done","output_index":1,"item":{"id":"done-second"}}),
	];
	let stream: String = input.into_iter().map(event).collect();
	let output = events(&normalize(stream).await);
	assert_eq!(output[2]["item_id"], "first");
	assert_eq!(output[3]["item"]["id"], "first");
	assert_eq!(output[4]["item_id"], "second");
	assert_eq!(output[5]["item"]["id"], "first");
	assert_eq!(output[6]["item"]["id"], "second");
}

#[tokio::test]
async fn copilot_responses_stream_emits_before_upstream_closes() {
	use futures_util::StreamExt;

	let input = event(
		json!({"type":"response.output_item.added","output_index":0,"item":{"id":"anchor"}}),
	) + &event(
		json!({"type":"response.output_text.delta","output_index":0,"item_id":"changed","delta":"early"}),
	);
	let body = Body::from_stream(
		futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from(input)) })
			.chain(futures_util::stream::pending()),
	);
	let mut output = normalize_responses_stream(body, LIMIT);
	let frame = tokio::time::timeout(std::time::Duration::from_secs(2), output.frame())
		.await
		.expect("complete SSE events must be emitted before upstream EOF")
		.unwrap()
		.unwrap()
		.into_data()
		.unwrap();
	assert_eq!(events(&frame)[1]["item_id"], "anchor");
}

#[tokio::test]
async fn copilot_responses_stream_preserves_metadata_and_fragmented_utf8() {
	let input = concat!(
		": heartbeat\r\nretry: 1000\r\nid: wire-id\r\nevent: response.output_item.added\r\n",
		"data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"anchor\"}}\r\n\r\n",
		"event: response.output_text.delta\r\n",
		"data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"item_id\":\"different\",\"delta\":\"天气🌤\",\"unknown\":{\"keep\":true}}\r\n\r\n",
		"event: unknown\r\ndata: not-json\r\n\r\ndata: [DONE]\r\n\r\n"
	);
	for (input, chunk_size) in [
		(input.to_owned(), input.len()),
		(input.replace("\r\n", "\n"), 1),
	] {
		let body = Body::from_stream(futures_util::stream::iter(
			input
				.as_bytes()
				.chunks(chunk_size)
				.map(|chunk| Ok::<_, std::io::Error>(Bytes::copy_from_slice(chunk)))
				.collect::<Vec<_>>(),
		));
		let output = normalize(body).await;
		let mut decoder = SseDecoder::<Bytes>::new();
		let mut bytes = BytesMut::from(output.as_ref());
		let mut frames = Vec::new();
		while let Some(frame) = decoder.decode_eof(&mut bytes).unwrap() {
			frames.push(frame);
		}
		assert!(
			frames
				.iter()
				.any(|frame| matches!(frame, Frame::Comment(comment) if comment.as_ref() == b"heartbeat"))
		);
		assert!(
			frames
				.iter()
				.any(|frame| matches!(frame, Frame::Retry(retry) if retry.as_millis() == 1000))
		);
		let frames: Vec<_> = frames
			.into_iter()
			.filter_map(|frame| match frame {
				Frame::Event(event) => Some(event),
				_ => None,
			})
			.collect();
		assert_eq!(frames[1].name, "response.output_text.delta");
		assert_eq!(frames[1].id.as_deref(), Some("wire-id"));
		let value: Value = serde_json::from_slice(&frames[1].data).unwrap();
		assert_eq!(value["item_id"], "anchor");
		assert_eq!(value["delta"], "天气🌤");
		assert_eq!(value["unknown"], json!({"keep":true}));
		assert_eq!(frames[2].name, "unknown");
		assert_eq!(frames[2].data.as_ref(), b"not-json");
		assert_eq!(frames[3].data.as_ref(), b"[DONE]");
	}
}

#[tokio::test]
async fn copilot_responses_stream_leaves_unregistered_and_unrelated_events_alone() {
	let input = [
		json!({"type":"response.output_text.delta","output_index":0,"item_id":"before-anchor","delta":"early"}),
		json!({"type":"response.output_item.added","output_index":0,"item":{"id":"anchor"}}),
		json!({"type":"response.output_text.delta","output_index":1,"item_id":"unregistered","delta":"other"}),
		json!({"type":"response.output_text.delta","item_id":"no-index","delta":"other"}),
		json!({"type":"other","output_index":0,"item_id":"unrelated"}),
		json!({"type":"response.output_text.delta","output_index":-1,"item_id":"negative"}),
		json!({"type":"response.output_text.delta","output_index":0,"item_id":null}),
		json!({"type":"response.output_text.delta","output_index":0,"item_id":"anchor","unknown":[1,2,3]}),
	];
	let stream: String = input.iter().cloned().map(event).collect();
	assert_eq!(events(&normalize(stream).await), input);
}

#[tokio::test]
async fn copilot_responses_stream_normalizes_all_terminal_snapshots() {
	for kind in [
		"response.completed",
		"response.failed",
		"response.incomplete",
	] {
		let stream =
			event(json!({"type":"response.output_item.added","output_index":0,"item":{"id":"anchor"}}))
				+ &event(
					json!({"type":kind,"response":{"output":[{"id":"different","encrypted_content":"opaque"}]}}),
				);
		let output = events(&normalize(stream).await);
		assert_eq!(
			output[1]["response"]["output"][0],
			json!({"id":"anchor","encrypted_content":"opaque"})
		);
	}
}

#[tokio::test]
async fn copilot_responses_stream_accepts_unterminated_done() {
	for done in ["data: [DONE]", "data: [DONE]\n", "data: [DONE]\r\n"] {
		let output = normalize(done).await;
		assert!(String::from_utf8_lossy(&output).contains("data: [DONE]"));
	}
}

#[tokio::test]
async fn copilot_responses_stream_enforces_frame_limit() {
	let input = event(json!({"type":"response.output_text.delta","delta":"x".repeat(1024)}));
	normalize_responses_stream(Body::from(input), 64)
		.collect()
		.await
		.expect_err("oversized SSE events must fail");
}

#[tokio::test]
async fn copilot_responses_stream_propagates_upstream_failure() {
	let input =
		event(json!({"type":"response.output_item.added","output_index":0,"item":{"id":"anchor"}}));
	let body = Body::from_stream(futures_util::stream::iter(vec![
		Ok(Bytes::from(input)),
		Err(std::io::Error::other("upstream failed")),
	]));
	let mut output = normalize_responses_stream(body, LIMIT);
	assert!(output.frame().await.unwrap().unwrap().is_data());
	assert!(output.frame().await.unwrap().is_err());
}
