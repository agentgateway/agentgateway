use std::collections::HashMap;
use std::convert::Infallible;

use ::http::HeaderMap;
use http_body_util::BodyExt;
use tokio_sse_codec::{Event, Frame, SseDecoder};

use super::*;
use crate::*;

#[tokio::test]
async fn test_parser() {
	let msg1 = "data: msg1\n\n";
	let msg2 = "data: msg2\n\n";
	let trailers = HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
	let body = http::Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
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
	let body = http::Body::from_stream(futures_util::stream::iter(vec![
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
async fn test_sse_json_transform() {
	let msg1 = "data: {\"msg\": 1, \"type\": \"input\"}\n\n";
	let msg2 = "data: {\"msg\": 2, \"type\": \"input\"}\n\n";
	let msg3 = "data: [DONE]\n\n";
	let trailers = HeaderMap::try_from(&HashMap::from([("k".to_string(), "v".to_string())])).unwrap();
	let body = http::Body::new(http_body_util::StreamBody::new(futures_util::stream::iter(
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

	// The result should contain the transformed SSE data
	let result_str = String::from_utf8_lossy(&result.to_bytes()).to_string();
	// TODO: fork or modify the library to not write empty events
	assert_eq!(
		result_str,
		r#"event: 
data: {"message":1,"error":"","status":"processed_input"}

event: 
data: {"message":2,"error":"","status":"processed_input"}

event: 
data: [DONE]

"#
	);
}
