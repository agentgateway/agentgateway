use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex};

use ::http::HeaderMap;
use agent_http::Body;
use bytes::{Bytes, BytesMut};
use http_body_util::BodyExt;
use rstest::rstest;
use serde::{Deserialize, Serialize};
use tokio_util::codec::Decoder;

use super::sse_decoder::{SseDecoder, SseFrame};
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
	let decoder = SseDecoder::new(usize::MAX);

	let events = Arc::new(Mutex::new(vec![]));
	let ev_clone = events.clone();
	let body = passthrough::parser(body, decoder, move |frame| {
		let SseFrame::Event(data) = frame else {
			panic!("unexpected {frame:?}");
		};
		events.lock().unwrap().push(data);
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

#[derive(Serialize)]
struct StatusOutput {
	status: &'static str,
}

#[tokio::test]
async fn test_sse_json() {
	let msg1 = r#"data: {"msg": 1}

"#;
	let msg2 = r#"data: {"msg": 2}

"#;
	let body = Body::from_stream(futures_util::stream::iter(vec![
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg1.as_bytes())),
		Ok::<_, std::io::Error>(Bytes::copy_from_slice(msg2.as_bytes())),
	]));
	let decoder = SseDecoder::new(usize::MAX);

	let events = Arc::new(Mutex::new(vec![]));
	let ev_clone = events.clone();
	let body = passthrough::parser(body, decoder, move |frame| {
		let SseFrame::Event(data) = frame else {
			panic!("unexpected {frame:?}");
		};
		events
			.lock()
			.unwrap()
			.push(serde_json::from_slice::<Test>(&data).unwrap());
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
	let msg1 = r#"data: {"msg": 1, "type": "input"}

"#;
	let msg2 = r#"data: {"msg": 2, "type": "input"}

"#;
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
	let msg1 = r#"data: {"msg": 1}

"#;
	let msg2 = r#"data: {"msg": 2}

"#;
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
			sse::SseJsonEvent::Eof | sse::SseJsonEvent::Error => Vec::new(),
		});

	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);
	assert!(
		result.contains("event: delta"),
		"missing named delta event:\n{result}"
	);
	assert!(
		result.contains(r#"data: {"message":1,"status":"ok"}"#),
		"missing translated payload for first event:\n{result}"
	);
	assert!(
		result.contains(r#"data: {"message":2,"status":"ok"}"#),
		"missing translated payload for second event:\n{result}"
	);
	assert!(
		result.contains("event: done"),
		"missing done event from [DONE] translation:\n{result}"
	);
	assert!(
		result.contains(r#"data: {"message":0,"status":"done"}"#),
		"missing done payload:\n{result}"
	);
}

#[tokio::test]
async fn test_sse_json_transform_multi_parse_error_path() {
	let msg1 = r#"data: {"msg": 1}

"#;
	let msg2 = r#"data: {"msg": "bad"}

"#;
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
			sse::SseJsonEvent::Eof | sse::SseJsonEvent::Error => Vec::new(),
		});

	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);
	assert!(
		result.contains("event: error"),
		"missing parse error event:\n{result}"
	);
	assert!(
		result.contains(r#"data: {"status":"parse_error"}"#),
		"missing parse error payload:\n{result}"
	);
	assert!(
		result.contains("event: done"),
		"missing done event after parse error:\n{result}"
	);
}

#[tokio::test]
async fn test_sse_json_transform_multi_reports_eof() {
	let body = Body::from("data: {\"msg\": 1}\n\n");
	let transformed =
		sse::json_transform_multi::<Test, StatusOutput, _>(body, 1024, |event| match event {
			sse::SseJsonEvent::Data(Ok(_)) => Vec::new(),
			sse::SseJsonEvent::Data(Err(_)) | sse::SseJsonEvent::Done => Vec::new(),
			sse::SseJsonEvent::Eof => vec![("eof", StatusOutput { status: "eof" })],
			sse::SseJsonEvent::Error => Vec::new(),
		});
	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);

	assert!(
		result.contains("event: eof"),
		"missing EOF event:\n{result}"
	);
	assert!(
		result.contains(r#"data: {"status":"eof"}"#),
		"missing EOF payload:\n{result}"
	);
}

#[tokio::test]
async fn test_sse_json_transform_multi_reports_body_error() {
	let body = Body::from_stream(futures_util::stream::iter(vec![Err::<Bytes, io::Error>(
		io::Error::other("upstream failed"),
	)]));
	let transformed = sse::json_transform_multi::<Test, StatusOutput, _>(body, 1024, |event| {
		if matches!(event, sse::SseJsonEvent::Error) {
			vec![("error", StatusOutput { status: "error" })]
		} else {
			Vec::new()
		}
	});
	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);

	assert!(
		result.contains("event: error"),
		"missing body error event:\n{result}"
	);
	assert!(result.contains(r#"data: {"status":"error"}"#));
}

#[tokio::test]
async fn test_sse_json_transform_multi_reports_decoder_error() {
	let body = Body::from("data: {\"msg\": 123456789}\n\n");
	let transformed = sse::json_transform_multi::<Test, StatusOutput, _>(body, 8, |event| {
		if matches!(event, sse::SseJsonEvent::Error) {
			vec![("error", StatusOutput { status: "error" })]
		} else {
			Vec::new()
		}
	});
	let result = transformed.collect().await.unwrap().to_bytes();
	let result = String::from_utf8_lossy(&result);

	assert!(
		result.contains("event: error"),
		"missing decoder error event:\n{result}"
	);
	assert!(result.contains(r#"data: {"status":"error"}"#));
}

#[tokio::test]
async fn test_sse_json_transform_multi_reports_truncated_frame_as_error() {
	let observed = Arc::new(Mutex::new(Vec::new()));
	let captured = observed.clone();
	let body = Body::from("data: {\"msg\":1}\n");
	let transformed =
		sse::json_transform_multi::<Test, StatusOutput, _>(body, 1024, move |event| match event {
			sse::SseJsonEvent::Error => {
				captured.lock().unwrap().push("error");
				vec![("error", StatusOutput { status: "error" })]
			},
			sse::SseJsonEvent::Eof => {
				captured.lock().unwrap().push("eof");
				Vec::new()
			},
			_ => Vec::new(),
		});
	let result = transformed.collect().await.unwrap().to_bytes();

	assert_eq!(*observed.lock().unwrap(), ["error"]);
	assert!(String::from_utf8_lossy(&result).contains(r#"data: {"status":"error"}"#));
}

#[tokio::test]
async fn test_sse_json_transform_multi_propagates_decoder_error_after_output() {
	let body = Body::from("data: {\"msg\":1}\n\ndata: {\"msg\":123456789}\n\n");
	let saw_error = Arc::new(Mutex::new(false));
	let captured_error = saw_error.clone();
	let mut transformed =
		sse::json_transform_multi::<Test, StatusOutput, _>(body, 20, move |event| match event {
			sse::SseJsonEvent::Data(Ok(_)) => vec![("delta", StatusOutput { status: "ok" })],
			sse::SseJsonEvent::Error => {
				*captured_error.lock().unwrap() = true;
				Vec::new()
			},
			_ => Vec::new(),
		});

	let first = transformed
		.frame()
		.await
		.expect("valid output must precede the decoder error")
		.expect("the first frame must contain the valid output");
	let data = first.data_ref().expect("the first frame must be data");
	assert!(String::from_utf8_lossy(data).contains(r#"data: {"status":"ok"}"#));

	transformed
		.frame()
		.await
		.expect("the decoder error must be returned")
		.expect_err("decoder error after valid output must propagate");
	assert!(*saw_error.lock().unwrap(), "decoder error was not observed");
}

#[tokio::test]
async fn test_sse_json_passthrough_unterminated_final_event() {
	for done in ["data: [DONE]", "data: [DONE]\n", "data: [DONE]\r\n"] {
		let input = format!("data: {{\"msg\":1}}\n\n{done}");

		let observed = Arc::new(Mutex::new(vec![]));
		let obs = observed.clone();
		let body = sse::json_passthrough::<Test>(Body::from(input.clone()), 1024, move |o| {
			obs.lock().unwrap().push(o.map(|r| r.unwrap().msg));
		});
		let got = body.collect().await.unwrap().to_bytes();
		assert_eq!(
			got,
			input.as_bytes(),
			"passthrough must forward bytes unmodified"
		);
		assert_eq!(*observed.lock().unwrap(), [Some(1), None], "{done:?}");
	}

	let body = sse::json_passthrough::<Test>(Body::from("data: {\"msg\":1}"), 1024, |_| {});
	body
		.collect()
		.await
		.expect_err("an unterminated non-[DONE] event must still fail");
}

#[rstest]
#[case::crlf_split_between_reads(
	&["data: {\"msg\":1}\r\n\r", "\ndata: {\"msg\":2}\r\n\r\n"],
	1024,
	&[Some(Ok(1)), Some(Ok(2))]
)]
#[case::bare_cr(&["data: {\"msg\":1}\r\rdata: {\"msg\":2}\r\r"], 1024, &[Some(Ok(1)), Some(Ok(2))])]
#[case::oversized_event_then_valid(
	&["data: {\"msg\":1,\"pad\":\"xxxxxxxxxxxxxxxx\"}\n\n", "data: {\"msg\":2}\n\n"],
	32,
	&[Some(Err(())), Some(Ok(2))]
)]
#[tokio::test]
async fn test_sse_json_passthrough_forwards_bytes_and_reports_events(
	#[case] chunks: &[&str],
	#[case] limit: usize,
	#[case] expected: &[Option<Result<u8, ()>>],
) {
	let body = Body::from_stream(futures_util::stream::iter(
		chunks
			.iter()
			.map(|c| Ok::<_, io::Error>(Bytes::copy_from_slice(c.as_bytes())))
			.collect::<Vec<_>>(),
	));
	let observed = Arc::new(Mutex::new(vec![]));
	let obs = observed.clone();
	let body = sse::json_passthrough::<Test>(body, limit, move |o| {
		obs
			.lock()
			.unwrap()
			.push(o.map(|r| r.map(|t| t.msg).map_err(|_| ())));
	});
	let got = body.collect().await.unwrap().to_bytes();
	assert_eq!(got, chunks.concat().as_bytes());
	assert_eq!(*observed.lock().unwrap(), expected);
}

#[tokio::test]
async fn test_sse_json_transform_multi_accepts_unterminated_final_done() {
	let observed = Arc::new(Mutex::new(Vec::new()));
	let captured = observed.clone();
	let body = Body::from("data: {\"msg\":1}\n\ndata: [DONE]");
	let transformed = sse::json_transform_multi::<Test, StatusOutput, _>(body, 1024, move |event| {
		captured.lock().unwrap().push(event);
		Vec::new()
	});
	transformed.collect().await.unwrap();
	let observed = std::mem::take(&mut *observed.lock().unwrap());
	assert!(
		matches!(
			observed[..],
			[
				sse::SseJsonEvent::Data(Ok(Test { msg: 1 })),
				sse::SseJsonEvent::Done,
				sse::SseJsonEvent::Eof
			]
		),
		"{observed:?}"
	);
}

#[rstest]
#[case::empty(&[], "[]")]
#[case::events(&["data: {\"a\":1}\n\n", "data: {\"a\":2}\n\n"], "[{\"a\":1},\r\n{\"a\":2}]")]
#[tokio::test]
async fn test_sse_to_json_array(#[case] chunks: &[&'static str], #[case] want: &str) {
	let body = Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
		chunks
			.iter()
			.map(|c| Ok::<_, Infallible>(http_body::Frame::data(Bytes::from_static(c.as_bytes()))))
			.collect::<Vec<_>>(),
	)));
	let got = sse::to_json_array(body, 1024)
		.collect()
		.await
		.unwrap()
		.to_bytes();
	assert_eq!(std::str::from_utf8(&got).unwrap(), want);
}
