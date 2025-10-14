use super::RewindSocket;
use crate::transport::stream::{Socket, TCPConnectionInfo};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

#[tokio::test]
async fn test_rewind() {
	let (mut client, mut rw) = setup_socket();

	client.write_all(b"hello").await.unwrap();
	let mut dest = vec![0u8; 1024];
	let _ = rw.read(&mut dest).await.unwrap();
	assert_eq!(&dest[..6], b"hello\0");
	rw.rewind();

	client.write_all(b" world").await.unwrap();
	let mut dest = vec![0u8; 1024];
	let _ = rw.read(&mut dest).await.unwrap();
	let _ = rw.read(&mut dest[5..]).await.unwrap();
	assert_eq!(&dest[..12], b"hello world\0");
}

#[tokio::test]
async fn test_discard() {
	let (mut client, mut rw) = setup_socket();

	client.write_all(b"hello").await.unwrap();
	let mut dest = vec![0u8; 1024];
	let _ = rw.read(&mut dest).await.unwrap();
	assert_eq!(&dest[..6], b"hello\0");
	let mut rw = rw.discard();

	client.write_all(b" world").await.unwrap();
	let mut dest = vec![0u8; 1024];
	let _ = rw.read(&mut dest).await.unwrap();
	assert_eq!(&dest[..7], b" world\0");
}

fn setup_socket() -> (DuplexStream, RewindSocket) {
	let (client, server) = tokio::io::duplex(8192);
	let base = Socket::from_memory(
		server,
		TCPConnectionInfo {
			peer_addr: "127.0.0.1:12345".parse().unwrap(),
			local_addr: "127.0.0.1:80".parse().unwrap(),
			start: Instant::now(),
		},
	);
	let (_ext, _counter, inner) = base.into_parts();
	let rw = RewindSocket::new(inner);
	(client, rw)
}
