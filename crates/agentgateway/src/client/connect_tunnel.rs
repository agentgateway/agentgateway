use crate::transport::stream::Socket;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn handshake(conn: &mut Socket, dest: &str) -> Result<(), anyhow::Error> {
	// While the raw HTTP/1 usage here looks pretty sketchy, hyper itself is doing this so its probably sufficient
	// for our simple needs here.
	// If we need to add TLS (which implies ALPN negotiation, etc) then we will want to make this more robust.
	let buf = format!(
		"\
         CONNECT {dest} HTTP/1.1\r\n\
         Host: {dest}\r\n\
         \r\n\
         "
	)
	.into_bytes();

	conn.write_all(&buf).await?;

	let mut buf = [0; 8192];
	let mut pos = 0;
	loop {
		let n = conn
			.read(&mut buf[pos..])
			.await
			.map_err(crate::http::Error::new)?;
		if n == 0 {
			return Err(anyhow::anyhow!("tunnel unexpected eof"));
		}
		pos += n;

		let recvd = &buf[..pos];
		if recvd.starts_with(b"HTTP/1.1 200") || recvd.starts_with(b"HTTP/1.0 200") {
			if recvd.ends_with(b"\r\n\r\n") {
				return Ok(());
			}
			if pos == buf.len() {
				return Err(anyhow::anyhow!("headers too long"));
			}
		// else read more
		} else if recvd.starts_with(b"HTTP/1.1 407") {
			return Err(anyhow::anyhow!("tunnel required auth"));
		} else {
			return Err(anyhow::anyhow!("tunnel failed"));
		}
	}
}
