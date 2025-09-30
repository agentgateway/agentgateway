use std::error::Error as StdErr;
use std::fmt;
use std::fmt::Formatter;

pub use client::*;
pub use metrics::*;
use tokio::sync::mpsc;
pub use types::*;

use self::service::discovery::v3::DeltaDiscoveryRequest;

mod client;
pub mod metrics;
mod types;

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("gRPC error {}", DisplayStatus(.0))]
	GrpcStatus(#[from] tonic::Status),
	#[error("gRPC connection error: {}", DisplayStatus(.0))]
	Connection(#[source] tonic::Status),
	#[error("connection failed {0}")]
	Transport(#[from] tonic::transport::Error),
	/// Attempted to send on a MPSC channel which has been canceled
	#[error(transparent)]
	RequestFailure(#[from] Box<mpsc::error::SendError<DeltaDiscoveryRequest>>),
	#[error("failed to send on demand resource")]
	OnDemandSend(),
}

struct DisplayStatus<'a>(&'a tonic::Status);

impl fmt::Display for DisplayStatus<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let s = &self.0;
		write!(f, "status: {:?}, message: {:?}", s.code(), s.message())?;

		if s.message().to_string().contains("authentication failure") {
			write!(
				f,
				" (hint: check the control plane logs for more information)"
			)?;
		}
		if !s.details().is_empty() {
			if let Ok(st) = std::str::from_utf8(s.details()) {
				write!(f, ", details: {st}")?;
			}
		}
		if let Some(src) = s.source().and_then(|s| s.source()) {
			write!(f, ", source: {src}")?;
			// Error is not public to explicitly match on, so do a fuzzy match
			if format!("{src}").contains("Temporary failure in name resolution") {
				write!(f, " (hint: is the DNS server reachable?)")?;
			}
		}
		Ok(())
	}
}
