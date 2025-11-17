// Mock Istio Certificate Service for testing HBONE mTLS
//
// Since rcgen doesn't support CSR parsing (https://github.com/rustls/rcgen/issues/228),
// we generate certificates with a static key and return the private key in the cert chain.
// This is a test-only approach - real CAs never return private keys.

use rand::RngCore;
use rcgen::{
	CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair,
	KeyUsagePurpose, SanType, SerialNumber,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tonic::{Request, Response, Status, transport::Server};

pub mod istio {
	pub mod ca {
		tonic::include_proto!("istio.v1.auth");
	}
}

use istio::ca::{
	IstioCertificateRequest, IstioCertificateResponse,
	istio_certificate_service_server::{IstioCertificateService, IstioCertificateServiceServer},
};

#[derive(Debug)]
pub struct MockCaService {
	ca_key: Arc<KeyPair>,
	ca_cert_pem: Arc<String>,
}

#[tonic::async_trait]
impl IstioCertificateService for MockCaService {
	async fn create_certificate(
		&self,
		_request: Request<IstioCertificateRequest>,
	) -> Result<Response<IstioCertificateResponse>, Status> {
		// We ignore the CSR since rcgen doesn't support parsing it
		// Instead, generate a certificate with a static key

		// Generate random serial number (159 bits)
		let serial_number = {
			let mut data = [0u8; 20];
			rand::rng().fill_bytes(&mut data);
			data[0] &= 0x7f;
			data
		};

		// Set up certificate parameters
		let mut params = CertificateParams::default();
		params.not_before = SystemTime::now().into();
		params.not_after = (SystemTime::now() + Duration::from_secs(365 * 24 * 60 * 60)).into();
		params.serial_number = Some(SerialNumber::from_slice(&serial_number));

		let mut dn = DistinguishedName::new();
		dn.push(DnType::OrganizationName, "cluster.local");
		params.distinguished_name = dn;

		params.key_usages = vec![
			KeyUsagePurpose::DigitalSignature,
			KeyUsagePurpose::KeyEncipherment,
		];
		params.extended_key_usages = vec![
			ExtendedKeyUsagePurpose::ServerAuth,
			ExtendedKeyUsagePurpose::ClientAuth,
		];

		// Set SPIFFE ID as SAN
		let spiffe_id = "spiffe://cluster.local/ns/default/sa/default";
		params.subject_alt_names =
			vec![SanType::URI(spiffe_id.try_into().map_err(|e| {
				Status::internal(format!("Failed to create SAN: {}", e))
			})?)];

		// Use static test key for consistency
		let kp = KeyPair::from_pem(std::str::from_utf8(super::shared_ca::TEST_PKEY).unwrap())
			.map_err(|e| Status::internal(format!("Failed to load test key: {}", e)))?;
		let key_pem = kp.serialize_pem();

		// Use the CA key
		let ca_kp = &*self.ca_key;

		// Sign the certificate with CA
		let issuer = Issuer::from_params(&params, &ca_kp);
		let cert = params
			.signed_by(&kp, &issuer)
			.map_err(|e| Status::internal(format!("Failed to sign certificate: {}", e)))?;
		let cert_pem = cert.pem();

		// For testing: return the private key in the cert chain so the client can use it
		// This is necessary because we can't parse the CSR to use its public key
		// We use a special marker to identify this as a test certificate
		const TEST_CERT_MARKER: &str = "X-Test-Certificate-Key";
		let cert_chain = vec![
			cert_pem,
			format!("# {}\n{}", TEST_CERT_MARKER, key_pem), // Test-only: include the private key with marker
			self.ca_cert_pem.to_string(),
		];

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
