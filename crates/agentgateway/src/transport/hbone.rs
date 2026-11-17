use std::fmt;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;

use agent_hbone::Key;
use async_trait::async_trait;

use crate::control::caclient::CaClient;
use crate::types::discovery::Identity;

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct WorkloadKey {
	pub dst_id: Vec<Identity>,
	pub dst: SocketAddr,
}

impl Key for WorkloadKey {
	fn dest(&self) -> SocketAddr {
		self.dst
	}
}

impl Display for WorkloadKey {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(f, "{}[", self.dst,)?;
		for i in &self.dst_id {
			write!(f, "{i}")?;
		}
		write!(f, "]")
	}
}

#[async_trait]
impl agent_hbone::pool::CertificateFetcher<WorkloadKey> for CaClient {
	async fn fetch_certificate(
		&self,
		key: WorkloadKey,
	) -> anyhow::Result<Arc<rustls::client::ClientConfig>> {
		let id = self.get_identity().await?;
		Ok(id.hbone_mtls(key.dst_id).connect_config())
	}
}
