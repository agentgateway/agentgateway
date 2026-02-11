// Mock Istio Certificate Service for testing HBONE mTLS
//
// Since rcgen doesn't support CSR parsing (https://github.com/rustls/rcgen/issues/228),
// we generate certificates with a static key and return the private key in the cert chain.
// This is a test-only approach - real CAs never return private keys.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rand::Rng;
use rcgen::{ExtendedKeyUsagePurpose, Issuer, KeyPair, KeyUsagePurpose, SanType, SerialNumber};
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
		// We ignore the CSR since rcgen doesn't support parsing it
		// Instead, generate a certificate with a static key
		// Parse the CSR to get the public key and subject info
		let csr_pem = req.into_inner().csr;

		// Generate random serial number (159 bits)
		let serial_number = {
			let mut data = [0u8; 20];
			rand::rng().fill_bytes(&mut data);
			data[0] &= 0x7f;
			data
		};

		// Set up certificate parameters from CSR
		let csr = rcgen::CertificateSigningRequestParams::from_pem(csr_pem.as_str())
			.map_err(|e| Status::internal(format!("Failed to parse CSR: {}", e)))?;
		let mut params = csr.params;

		params.not_before = SystemTime::now().into();
		params.not_after = (SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60)).into();
		params.serial_number = Some(SerialNumber::from_slice(&serial_number));

		params.key_usages = vec![
			KeyUsagePurpose::DigitalSignature,
			KeyUsagePurpose::KeyEncipherment,
		];
		params.extended_key_usages = vec![
			ExtendedKeyUsagePurpose::ServerAuth,
			ExtendedKeyUsagePurpose::ClientAuth,
		];

		// Set SPIFFE ID as SAN (you may want to extract this from the CSR instead)
		let spiffe_id = "spiffe://cluster.local/ns/default/sa/default";
		params.subject_alt_names =
			vec![SanType::URI(spiffe_id.try_into().map_err(|e| {
				Status::internal(format!("Failed to create SAN: {}", e))
			})?)];

		// Use the CA key to sign
		let ca_kp = &*self.ca_key;
		let issuer = Issuer::from_ca_cert_pem(&self.ca_cert_pem, ca_kp)
			.map_err(|e| Status::internal(format!("Failed to load CA cert: {}", e)))?;

		// Sign the certificate with CA using the public key from the CSR
		let cert = params
			.signed_by(&csr.public_key, &issuer)
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
