use std::io;

use futures_util::stream;
use http_body_util::{BodyExt, StreamBody};

use super::*;

#[derive(Default)]
struct DelayedDecoder {
	saw_data: bool,
}

impl Decoder for DelayedDecoder {
	type Item = Bytes;
	type Error = io::Error;

	fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
		if src.is_empty() {
			return Ok(None);
		}
		if !self.saw_data {
			self.saw_data = true;
			return Ok(None);
		}
		Ok(Some(src.split().freeze()))
	}
}

struct NoopEncoder;

impl Encoder<Bytes> for NoopEncoder {
	type Error = io::Error;

	fn encode(&mut self, _item: Bytes, _dst: &mut BytesMut) -> Result<(), Self::Error> {
		Ok(())
	}
}

#[tokio::test]
async fn strict_parser_preserves_buffered_trailers_when_handler_terminates() {
	let mut trailers = HeaderMap::new();
	trailers.insert("x-test-trailer", "value".parse().unwrap());
	let frames = vec![
		Ok::<_, io::Error>(http_body::Frame::data(Bytes::from_static(b"data"))),
		Ok(http_body::Frame::trailers(trailers.clone())),
	];
	let body = AxumBody::new(StreamBody::new(stream::iter(frames)));
	let transformed = strict_parser(body, DelayedDecoder::default(), NoopEncoder, |_| {
		(Vec::<Bytes>::new(), true)
	});

	let collected = transformed.collect().await.unwrap();

	assert_eq!(collected.trailers(), Some(&trailers));
}
