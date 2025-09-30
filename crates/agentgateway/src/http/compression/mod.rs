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
use tokio_util::io::StreamReader;

const GZIP: &str = "gzip";
const DEFLATE: &str = "deflate";
const BR: &str = "br";
const ZSTD: &str = "zstd";

pub async fn to_bytes_with_decompression(
	body: axum_core::body::Body,
	encoding: Option<ContentEncoding>,
	limit: usize,
) -> Result<(Option<&'static str>, Bytes), axum_core::Error> {
	match encoding {
		Some(c) if c.contains(GZIP) => Ok((Some(GZIP), decode_body(body, GZIP, limit).await?)),
		Some(c) if c.contains(DEFLATE) => Ok((Some(DEFLATE), decode_body(body, DEFLATE, limit).await?)),
		Some(c) if c.contains(BR) => Ok((Some(BR), decode_body(body, BR, limit).await?)),
		Some(c) if c.contains(ZSTD) => Ok((Some(ZSTD), decode_body(body, ZSTD, limit).await?)),
		// TODO: explicitly error on Some() that we don't know about?
		_ => Ok((None, crate::http::read_body_with_limit(body, limit).await?)),
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
