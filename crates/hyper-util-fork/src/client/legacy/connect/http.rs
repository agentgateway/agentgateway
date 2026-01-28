use std::net::SocketAddr;

use tokio::net::TcpStream;

use super::{Connected, Connection};
use crate::rt::TokioIo;

/// Extra information about the transport when an HttpConnector is used.
///
/// # Example
///
/// ```ignore
/// # fn doc(res: http::Response<()>) {
/// use hyper_util::client::legacy::connect::HttpInfo;
///
/// // res = http::Response
/// res
///     .extensions()
///     .get::<HttpInfo>()
///     .map(|info| {
///         println!("remote addr = {}", info.remote_addr());
///     });
/// # }
/// ```
///
/// # Note
///
/// If a different connector is used besides [`HttpConnector`](HttpConnector),
/// this value will not exist in the extensions. Consult that specific
/// connector to see what "extra" information it might provide to responses.
#[derive(Clone, Debug)]
pub struct HttpInfo {
	remote_addr: SocketAddr,
	local_addr: SocketAddr,
}

impl Connection for TcpStream {
	fn connected(&self) -> Connected {
		let connected = Connected::new();
		if let (Ok(remote_addr), Ok(local_addr)) = (self.peer_addr(), self.local_addr()) {
			connected.extra(HttpInfo {
				remote_addr,
				local_addr,
			})
		} else {
			connected
		}
	}
}

#[cfg(unix)]
impl Connection for tokio::net::UnixStream {
	fn connected(&self) -> Connected {
		Connected::new()
	}
}

#[cfg(windows)]
impl Connection for tokio::net::windows::named_pipe::NamedPipeClient {
	fn connected(&self) -> Connected {
		Connected::new()
	}
}

// Implement `Connection` for generic `TokioIo<T>` so that external crates can
// implement their own `HttpConnector` with `TokioIo<CustomTcpStream>`.
impl<T> Connection for TokioIo<T>
where
	T: Connection,
{
	fn connected(&self) -> Connected {
		self.inner().connected()
	}
}

impl HttpInfo {
	/// Get the remote address of the transport used.
	pub fn remote_addr(&self) -> SocketAddr {
		self.remote_addr
	}

	/// Get the local address of the transport used.
	pub fn local_addr(&self) -> SocketAddr {
		self.local_addr
	}
}
