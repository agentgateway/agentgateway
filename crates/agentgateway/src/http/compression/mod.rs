use async_compression::tokio::bufread::{
	BrotliDecoder, BrotliEncoder, GzipDecoder, GzipEncoder, ZlibDecoder, ZlibEncoder, ZstdDecoder,
	ZstdEncoder,
};
use bytes::Bytes;
use futures_util::TryStreamExt;
use headers::ContentEncoding;
use http_body::Body;
use http_body_util::BodyExt;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio_util::io::{ReaderStream, StreamReader};

const GZIP: &str = "gzip";
const DEFLATE: &str = "deflate";
const BR: &str = "br";
const ZSTD: &str = "zstd";

/// Errors that can occur during compression/decompression operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unsupported content encoding")]
	UnsupportedEncoding,
	#[error("body exceeded buffer limit")]
	LimitExceeded,
	#[error("decompression failed: {0}")]
	Io(#[from] std::io::Error),
	#[error("body read error: {0}")]
	Body(#[from] axum_core::Error),
}

impl From<Error> for axum_core::Error {
	fn from(e: Error) -> Self {
		axum_core::Error::new(e)
	}
}

/// Detects which encoding is present in the Content-Encoding header.
fn detect_encoding(ce: &ContentEncoding) -> Option<&'static str> {
	if ce.contains(GZIP) {
		Some(GZIP)
	} else if ce.contains(DEFLATE) {
		Some(DEFLATE)
	} else if ce.contains(BR) {
		Some(BR)
	} else if ce.contains(ZSTD) {
		Some(ZSTD)
	} else {
		None
	}
}

/// Decompresses an HTTP body stream, returning a new body that yields decompressed chunks.
///
/// Use this for streaming responses (SSE, large files) where you can't buffer the entire body.
/// If encoding is None, returns the body unchanged.
/// If encoding is Some but unsupported, returns an error.
pub fn decompress_body<B>(
	body: B,
	encoding: Option<&ContentEncoding>,
) -> Result<axum_core::body::Body, Error>
where
	B: Body<Data = Bytes> + Send + Unpin + 'static,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	match encoding {
		None => Ok(axum_core::body::Body::new(body)),
		Some(ce) => match detect_encoding(ce) {
			Some(enc) => Ok(decompress_body_with_encoding(body, enc)),
			None => Err(Error::UnsupportedEncoding),
		},
	}
}

fn decompress_body_with_encoding<B>(body: B, encoding: &str) -> axum_core::body::Body
where
	B: Body + Send + Unpin + 'static,
	B::Data: Send,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	let byte_stream = body.into_data_stream().map_err(std::io::Error::other);
	let stream_reader = BufReader::new(StreamReader::new(byte_stream));

	let decoder: Box<dyn AsyncRead + Unpin + Send> = match encoding {
		GZIP => Box::new(GzipDecoder::new(stream_reader)),
		DEFLATE => Box::new(ZlibDecoder::new(stream_reader)),
		BR => Box::new(BrotliDecoder::new(stream_reader)),
		ZSTD => Box::new(ZstdDecoder::new(stream_reader)),
		unknown => panic!("unknown decoder: {unknown}"),
	};

	axum_core::body::Body::from_stream(ReaderStream::new(decoder))
}

pub async fn to_bytes_with_decompression(
	body: axum_core::body::Body,
	encoding: Option<&ContentEncoding>,
	limit: usize,
) -> Result<(Option<&'static str>, Bytes), Error> {
	match encoding {
		None => {
			let byte_stream = TryStreamExt::map_err(body.into_data_stream(), std::io::Error::other);
			let stream_reader = StreamReader::new(byte_stream);
			Ok((None, read_to_bytes(stream_reader, limit).await?))
		},
		Some(ce) => match detect_encoding(ce) {
			Some(enc) => Ok((Some(enc), decode_body(body, enc, limit).await?)),
			None => Err(Error::UnsupportedEncoding),
		},
	}
}

pub async fn encode_body(body: &[u8], encoding: &str) -> Result<Bytes, axum_core::Error> {
	let reader = BufReader::new(body);

	let encoder: Box<dyn tokio::io::AsyncRead + Unpin + Send> = match encoding {
		GZIP => Box::new(GzipEncoder::new(reader)),
		DEFLATE => Box::new(ZlibEncoder::new(reader)),
		BR => Box::new(BrotliEncoder::new(reader)),
		ZSTD => Box::new(ZstdEncoder::new(reader)),
		unknown => panic!("unknown encoder: {unknown}"),
	};

	// Use usize::MAX since encoding has no limit, convert Error to axum_core::Error
	read_to_bytes(encoder, usize::MAX).await.map_err(Into::into)
}

async fn decode_body<B>(body: B, encoding: &str, limit: usize) -> Result<Bytes, Error>
where
	B: Body + Send + Unpin + 'static,
	B::Data: Send,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	let byte_stream = body.into_data_stream().map_err(std::io::Error::other);

	let stream_reader = BufReader::new(StreamReader::new(byte_stream));

	let decoder: Box<dyn AsyncRead + Unpin + Send> = match encoding {
		GZIP => Box::new(GzipDecoder::new(stream_reader)),
		DEFLATE => Box::new(ZlibDecoder::new(stream_reader)),
		BR => Box::new(BrotliDecoder::new(stream_reader)),
		ZSTD => Box::new(ZstdDecoder::new(stream_reader)),
		unknown => panic!("unknown decoder: {unknown}"),
	};

	read_to_bytes(decoder, limit).await
}

async fn read_to_bytes<R>(mut reader: R, limit: usize) -> Result<Bytes, Error>
where
	R: AsyncRead + Unpin,
{
	let mut buffer = bytes::BytesMut::new();
	loop {
		let n = reader.read_buf(&mut buffer).await?;
		if buffer.len() > limit {
			return Err(Error::LimitExceeded);
		}
		if n == 0 {
			break;
		}
	}
	Ok(buffer.freeze())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Body;
	use headers::HeaderMapExt;

	#[tokio::test]
	async fn test_decompress_unsupported() {
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("unsupported"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let result = decompress_body(body, Some(&ce));
		assert!(matches!(result, Err(Error::UnsupportedEncoding)));
	}

	#[tokio::test]
	async fn test_to_bytes_limit_exceeded() {
		let body = Body::from("this is too long");
		let result = to_bytes_with_decompression(body, None, 5).await;
		assert!(matches!(result, Err(Error::LimitExceeded)));
	}

	#[tokio::test]
	async fn test_to_bytes_unsupported() {
		let body = Body::from("hello");
		let mut headers = crate::http::HeaderMap::new();
		headers.insert(
			crate::http::header::CONTENT_ENCODING,
			crate::http::HeaderValue::from_static("unsupported"),
		);
		let ce = headers.typed_get::<ContentEncoding>().unwrap();
		let result = to_bytes_with_decompression(body, Some(&ce), 100).await;
		assert!(matches!(result, Err(Error::UnsupportedEncoding)));
	}
}
