use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use ::http::HeaderMap;
use axum_core::body::Body;
use bytes::{Bytes, BytesMut};
use http_body::Body as _;
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use tokio_sse_codec::{Event, Frame, SseDecoder};
use tokio_util::codec::Decoder;

use super::{passthrough, sse};

#[tokio::test]
async fn test_parser() {
	let msg1 = "data: msg1\n\n";
	let msg2 = "data: msg2\n\n";
	let trailers = HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
	let body = Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
		vec![
			Ok::<_, Infallible>(http_body::Frame::data(Bytes::copy_from_slice(
				msg1.as_bytes(),
			))),
			Ok::<_, Infallible>(http_body::Frame::data(Bytes::copy_from_slice(
				msg2.as_bytes(),
			))),
			Ok::<_, Infallible>(http_body::Frame::trailers(trailers.clone())),
		],
	)));
	let decoder = SseDecoder::<Bytes>::new();

	let events = Arc::new(Mutex::new(vec![]));
	let ev_clone = events.clone();
	let body = passthrough::parser(body, decoder, move |o| match o {
		Frame::Comment(_) => {},
		Frame::Event(Event::<Bytes> { data, .. }) => {
			events.clone().lock().unwrap().push(data);
		},
		Frame::Retry(_) => {},
	});
	let got = body.collect().await.unwrap();
	assert_eq!(Some(&trailers), got.trailers());
	let got = got.to_bytes();
	assert_eq!(
		got,
		Bytes::copy_from_slice(format!("{msg1}{msg2}").as_bytes())
	);
	assert_eq!(
		ev_clone.lock().unwrap().clone(),
		vec![
			Bytes::copy_from_slice(b"msg1"),
			Bytes::copy_from_slice(b"msg2"),
		]
	);
}

#[derive(Clone, Eq, PartialEq, Debug, Deserialize)]
struct Test {
	msg: u8,
}

#[tokio::test]
async fn test_sse_json() {
	let msg1 = "data: {\"msg\": 1}\n\n";
	let msg2 = "data: {\"msg\": 2}\n\n";
	let body = Body::from_stream(futures_util::stream::iter(vec![
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg1.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg2.as_bytes())),
	]));
	let decoder = SseDecoder::<Bytes>::new();

	let events = Arc::new(Mutex::new(vec![]));
	let ev_clone = events.clone();
	let body = passthrough::parser(body, decoder, move |o| {
		events
			.clone()
			.lock()
			.unwrap()
			.push(sse::unwrap_json::<Test>(o).unwrap().unwrap())
	});
	let got = body.collect().await.map(|col| col.to_bytes()).unwrap();
	assert_eq!(
		got,
		Bytes::copy_from_slice(format!("{msg1}{msg2}").as_bytes())
	);
	assert_eq!(
		ev_clone.lock().unwrap().clone(),
		vec![Test { msg: 1 }, Test { msg: 2 },]
	);
}

#[tokio::test]
async fn test_full_passthrough_parser_flushes_decoder_on_eof() {
	struct EofOnlyDecoder;

	impl Decoder for EofOnlyDecoder {
		type Item = Bytes;
		type Error = io::Error;

		fn decode(&mut self, _src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
			Ok(None)
		}

		fn decode_eof(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
			if src.is_empty() {
				Ok(None)
			} else {
				Ok(Some(src.split().freeze()))
			}
		}
	}

	let msg = Bytes::from_static(b"tail");
	let body = Body::from_stream(futures_util::stream::iter(vec![Ok::<_, io::Error>(
		msg.clone(),
	)]));

	let events = Arc::new(Mutex::new(vec![]));
	let ev_clone = events.clone();
	let body = passthrough::full_passthrough_parser(body, EofOnlyDecoder, move |o| {
		events.clone().lock().unwrap().push(o)
	});
	let got = body.collect().await.map(|col| col.to_bytes()).unwrap();
	assert_eq!(got, msg);
	assert_eq!(ev_clone.lock().unwrap().clone(), vec![msg]);
}

#[tokio::test]
async fn test_sse_json_transform() {
	let msg1 = "data: {\"msg\": 1, \"type\": \"input\"}\n\n";
	let msg2 = "data: {\"msg\": 2, \"type\": \"input\"}\n\n";
	let msg3 = "data: [DONE]\n\n";
	let trailers = HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
	let body = Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
		vec![
			Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(
				msg1.as_bytes(),
			))),
			Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(
				msg2.as_bytes(),
			))),
			Ok::<_, std::io::Error>(http_body::Frame::data(Bytes::copy_from_slice(
				msg3.as_bytes(),
			))),
			Ok::<_, std::io::Error>(http_body::Frame::trailers(trailers.clone())),
		],
	)));

	#[derive(Deserialize)]
	struct Input {
		msg: u8,
		#[serde(rename = "type")]
		type_: String,
	}

	#[derive(Serialize)]
	struct Output {
		message: u8,
		error: String,
		status: String,
	}

	let transformed_body = sse::json_transform::<Input, Output>(body, 1024, |input| match input {
		Ok(input) => Some(Output {
			message: input.msg,
			error: "".to_string(),
			status: format!("processed_{}", input.type_),
		}),
		Err(e) => Some(Output {
			message: 0,
			error: e.to_string(),
			status: "error".to_string(),
		}),
	});

	let result = transformed_body.collect().await.unwrap();
	assert_eq!(Some(&trailers), result.trailers());

	let result_str = String::from_utf8_lossy(&result.to_bytes()).to_string();
	assert_eq!(
		result_str,
		r#"data: {"message":1,"error":"","status":"processed_input"}

data: {"message":2,"error":"","status":"processed_input"}

data: [DONE]

"#
	);
}

#[tokio::test]
async fn test_sse_json_transform_multi_named_events_and_done() {
	let msg1 = "data: {\"msg\": 1}\n\n";
	let msg2 = "data: {\"msg\": 2}\n\n";
	let done = "data: [DONE]\n\n";
	let body = Body::from_stream(futures_util::stream::iter(vec![
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg1.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg2.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(done.as_bytes())),
	]));

	#[derive(Deserialize)]
	struct Input {
		msg: u8,
	}
	#[derive(Serialize)]
	struct Output {
		message: u8,
		status: &'static str,
	}

	let transformed =
		sse::json_transform_multi::<Input, Output, _>(body, 1024, |event| match event {
			sse::SseJsonEvent::Data(Ok(input)) => vec![(
				"delta",
				Output {
					message: input.msg,
					status: "ok",
				},
			)],
			sse::SseJsonEvent::Data(Err(_)) => vec![(
				"error",
				Output {
					message: 0,
					status: "parse_error",
				},
			)],
			sse::SseJsonEvent::Done => vec![(
				"done",
				Output {
					message: 0,
					status: "done",
				},
			)],
		});

	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);
	assert!(
		result.contains("event: delta"),
		"missing named delta event:\n{result}"
	);
	assert!(
		result.contains("data: {\"message\":1,\"status\":\"ok\"}"),
		"missing translated payload for first event:\n{result}"
	);
	assert!(
		result.contains("data: {\"message\":2,\"status\":\"ok\"}"),
		"missing translated payload for second event:\n{result}"
	);
	assert!(
		result.contains("event: done"),
		"missing done event from [DONE] translation:\n{result}"
	);
	assert!(
		result.contains("data: {\"message\":0,\"status\":\"done\"}"),
		"missing done payload:\n{result}"
	);
}

#[tokio::test]
async fn test_sse_json_transform_multi_parse_error_path() {
	let msg1 = "data: {\"msg\": 1}\n\n";
	let msg2 = "data: {\"msg\": \"bad\"}\n\n";
	let done = "data: [DONE]\n\n";
	let body = Body::from_stream(futures_util::stream::iter(vec![
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg1.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg2.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(done.as_bytes())),
	]));

	#[derive(Deserialize)]
	struct Input {
		msg: u8,
	}
	#[derive(Serialize)]
	struct Output {
		status: &'static str,
	}

	let transformed =
		sse::json_transform_multi::<Input, Output, _>(body, 1024, |event| match event {
			sse::SseJsonEvent::Data(Ok(input)) => {
				let _ = input.msg;
				vec![("delta", Output { status: "ok" })]
			},
			sse::SseJsonEvent::Data(Err(_)) => vec![(
				"error",
				Output {
					status: "parse_error",
				},
			)],
			sse::SseJsonEvent::Done => vec![("done", Output { status: "done" })],
		});

	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);
	assert!(
		result.contains("event: error"),
		"missing parse error event:\n{result}"
	);
	assert!(
		result.contains("data: {\"status\":\"parse_error\"}"),
		"missing parse error payload:\n{result}"
	);
	assert!(
		result.contains("event: done"),
		"missing done event after parse error:\n{result}"
	);
}

#[tokio::test]
async fn test_sse_strict_named_json_frame_then_physical_eof_once() {
	let body = Body::from("event: content_block_delta\ndata: {\"msg\": 1}\n\n");
	let events = Arc::new(Mutex::new(vec![]));
	let events_clone = events.clone();

	let transformed =
		sse::json_transform_strict_with_eof::<Test, serde_json::Value>(body, 1024, move |event| {
			let seen = match event {
				sse::StrictSseJsonEvent::Data { event_name, data } => {
					assert_eq!(data.unwrap(), Test { msg: 1 });
					format!("data:{}", event_name.unwrap())
				},
				sse::StrictSseJsonEvent::Done => "done".to_string(),
				sse::StrictSseJsonEvent::Eof => "eof".to_string(),
				sse::StrictSseJsonEvent::TransportError => "error".to_string(),
			};
			events_clone.lock().unwrap().push(seen);
			(Vec::<(&'static str, serde_json::Value)>::new(), false)
		});

	transformed.collect().await.unwrap();
	assert_eq!(
		events.lock().unwrap().as_slice(),
		["data:content_block_delta", "eof"]
	);
}

#[test]
fn test_sse_strict_yields_after_one_nonempty_handler_batch_per_poll() {
	let body = Body::from(
		[
			"event: delta\ndata: {\"msg\": 1}\n\n",
			"event: delta\ndata: {\"msg\": 2}\n\n",
			"event: delta\ndata: {\"msg\": 3}\n\n",
		]
		.concat(),
	);
	let handled = Arc::new(Mutex::new(0));
	let handled_clone = handled.clone();
	let mut transformed =
		sse::json_transform_strict_with_eof::<Test, serde_json::Value>(body, 1024, move |event| {
			match event {
				sse::StrictSseJsonEvent::Data { data, .. } => {
					*handled_clone.lock().unwrap() += 1;
					(
						vec![("delta", serde_json::json!({"msg": data.unwrap().msg}))],
						false,
					)
				},
				_ => (Vec::new(), false),
			}
		});
	let waker = futures_util::task::noop_waker();
	let mut cx = Context::from_waker(&waker);
	assert!(matches!(
		Pin::new(&mut transformed).poll_frame(&mut cx),
		Poll::Pending
	));
	assert_eq!(*handled.lock().unwrap(), 0);
	let Poll::Ready(Some(Ok(frame))) = Pin::new(&mut transformed).poll_frame(&mut cx) else {
		panic!("second poll should yield translated data")
	};
	assert!(frame.data_ref().is_some_and(|data| !data.is_empty()));
	assert_eq!(*handled.lock().unwrap(), 1);
}
