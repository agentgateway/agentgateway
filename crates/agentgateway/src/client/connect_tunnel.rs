use http::HeaderValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::transport::stream::Socket;

pub async fn handshake(
	conn: Socket,
	dest: &str,
	auth: Option<HeaderValue>,
) -> Result<Socket, anyhow::Error> {
	let (ext, metrics, inner) = conn.into_parts();
	let mut conn = Socket::new_rewind(inner);
	// While the raw HTTP/1 usage here looks pretty sketchy, hyper itself is doing this so its probably sufficient
	// for our simple needs here.
	// If we need to add TLS (which implies ALPN negotiation, etc) then we will want to make this more robust.
	let mut buf = format!(
		"\
         CONNECT {dest} HTTP/1.1\r\n\
         Host: {dest}\r\n\
         "
	)
	.into_bytes();

	if let Some(auth) = auth {
		buf.extend_from_slice(b"Proxy-Authorization: ");
		buf.extend_from_slice(auth.as_bytes());
		buf.extend_from_slice(b"\r\n");
	}
	// headers end
	buf.extend_from_slice(b"\r\n");
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

		if let Some(end) = header_end(&buf[..pos]) {
			let recvd = &buf[..pos];
			if recvd.starts_with(b"HTTP/1.1 200") || recvd.starts_with(b"HTTP/1.0 200") {
				let conn = conn.keep_after(end);
				return Ok(Socket::from_rewind(ext, metrics, conn));
			} else if recvd.starts_with(b"HTTP/1.1 407") || recvd.starts_with(b"HTTP/1.0 407") {
				return Err(anyhow::anyhow!("tunnel required auth"));
			} else {
				return Err(anyhow::anyhow!("tunnel failed"));
			}
		}
		if pos == buf.len() {
			return Err(anyhow::anyhow!("headers too long"));
		}
	}
}

fn header_end(buf: &[u8]) -> Option<usize> {
	buf
		.windows(4)
		.position(|w| w == b"\r\n\r\n")
		.map(|pos| pos + 4)
}

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};
	use std::time::Instant;

	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	use super::*;
	use crate::transport::stream::TCPConnectionInfo;

	fn memory_socket(stream: tokio::io::DuplexStream) -> Socket {
		Socket::from_memory(
			stream,
			TCPConnectionInfo {
				peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1234),
				local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4321),
				start: Instant::now(),
				raw_peer_addr: None,
			},
		)
	}

	#[tokio::test]
	async fn handshake_replays_bytes_after_connect_headers() {
		let (client, mut server) = tokio::io::duplex(1024);
		let server_task = tokio::spawn(async move {
			let mut request = vec![0; 256];
			let n = server.read(&mut request).await.expect("read request");
			assert!(
				std::str::from_utf8(&request[..n])
					.unwrap()
					.starts_with("CONNECT dest:443 ")
			);
			server
				.write_all(b"HTTP/1.1 200 OK\r\n\r\nhello")
				.await
				.expect("write response");
		});

		let mut tunneled = handshake(memory_socket(client), "dest:443", None)
			.await
			.expect("handshake should succeed");
		let mut first_bytes = [0; 5];
		tunneled
			.read_exact(&mut first_bytes)
			.await
			.expect("read replayed bytes");

		assert_eq!(&first_bytes, b"hello");
		server_task.await.expect("server task");
	}
}
