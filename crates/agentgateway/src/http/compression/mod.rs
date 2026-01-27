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
/// If encoding is None or unsupported, returns the body unchanged.
pub fn decompress_body<B>(body: B, encoding: Option<ContentEncoding>) -> axum_core::body::Body
where
	B: Body<Data = Bytes> + Send + Unpin + 'static,
	B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
	match encoding.as_ref().and_then(detect_encoding) {
		Some(enc) => decompress_body_with_encoding(body, enc),
		None => axum_core::body::Body::new(body),
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
	encoding: Option<ContentEncoding>,
	limit: usize,
) -> Result<(Option<&'static str>, Bytes), axum_core::Error> {
	match encoding.as_ref().and_then(detect_encoding) {
		Some(enc) => Ok((Some(enc), decode_body(body, enc, limit).await?)),
		// TODO: explicitly error on Some() that we don't know about?
		None => Ok((None, crate::http::read_body_with_limit(body, limit).await?)),
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

	read_to_bytes(encoder, usize::MAX).await
}

async fn decode_body<B>(body: B, encoding: &str, limit: usize) -> Result<Bytes, axum_core::Error>
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

async fn read_to_bytes<R>(mut reader: R, limit: usize) -> Result<Bytes, axum_core::Error>
where
	R: AsyncRead + Unpin,
{
	let mut buffer = bytes::BytesMut::new();
	loop {
		let n = reader
			.read_buf(&mut buffer)
			.await
			.map_err(axum_core::Error::new)?;
		if buffer.len() > limit {
			return Err(axum_core::Error::new(anyhow::anyhow!(
				"exceeded buffer size"
			)));
		}
		if n == 0 {
			break;
		}
	}
	Ok(buffer.freeze())
}
