// Mock Istio Certificate Service for testing HBONE mTLS

use std::net::SocketAddr;
use std::sync::Arc;

use rcgen::{CertificateSigningRequestParams, Issuer, KeyPair};
use tonic::transport::Server;
use tonic::{Request, Response, Status};

pub mod istio {
	pub mod ca {
		tonic::include_proto!("istio.v1.auth");
	}
}

use istio::ca::istio_certificate_service_server::{
	IstioCertificateService, IstioCertificateServiceServer,
};
use istio::ca::{IstioCertificateRequest, IstioCertificateResponse};

#[derive(Debug)]
pub struct MockCaService {
	ca_key: Arc<KeyPair>,
	ca_cert_pem: Arc<String>,
}

#[tonic::async_trait]
impl IstioCertificateService for MockCaService {
	async fn create_certificate(
		&self,
		req: Request<IstioCertificateRequest>,
	) -> Result<Response<IstioCertificateResponse>, Status> {
		// Parse the CSR from the request
		let csr_pem = req.into_inner().csr;
		let csr = CertificateSigningRequestParams::from_pem(&csr_pem)
			.map_err(|e| Status::internal(format!("Failed to parse CSR: {}", e)))?;

		// Sign with CA issuer
		let issuer = Issuer::from_ca_cert_pem(self.ca_cert_pem.as_str(), &*self.ca_key)
			.map_err(|e| Status::internal(format!("Failed to load CA issuer: {}", e)))?;
		let cert = csr
			.signed_by(&issuer)
			.map_err(|e| Status::internal(format!("Failed to sign certificate: {}", e)))?;

		let cert_pem = cert.pem();
		let cert_chain = vec![cert_pem, self.ca_cert_pem.to_string()];

		Ok(Response::new(IstioCertificateResponse { cert_chain }))
	}
}

pub async fn start_mock_ca_server() -> anyhow::Result<SocketAddr> {
	let shared_ca = super::shared_ca::get_shared_ca();

	let addr = SocketAddr::from(([127, 0, 0, 1], 0));
	let listener = tokio::net::TcpListener::bind(addr).await?;
	let addr = listener.local_addr()?;

	let ca_service = MockCaService {
		ca_key: shared_ca.ca_key.clone(),
		ca_cert_pem: shared_ca.ca_cert_pem.clone(),
	};

	tokio::spawn(async move {
		Server::builder()
			.add_service(IstioCertificateServiceServer::new(ca_service))
			.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
			.await
			.expect("CA server failed");
	});

	// The listener is already bound and listening, so the server is ready
	Ok(addr)
}
