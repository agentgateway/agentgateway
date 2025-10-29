// Mock Istio Certificate Service for testing HBONE mTLS

use openssl::asn1::Asn1Time;
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::x509::extension::{KeyUsage, SubjectAlternativeName};
use openssl::x509::{X509, X509Builder};
use std::net::SocketAddr;
use std::sync::Arc;
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
	ca_key: Arc<PKey<Private>>,
	ca_cert: Arc<X509>,
}

#[tonic::async_trait]
impl IstioCertificateService for MockCaService {
	async fn create_certificate(
		&self,
		request: Request<IstioCertificateRequest>,
	) -> Result<Response<IstioCertificateResponse>, Status> {
		let req = request.into_inner();

		let csr = openssl::x509::X509Req::from_pem(req.csr.as_bytes())
			.map_err(|e| Status::invalid_argument(format!("Invalid CSR: {}", e)))?;

		let pubkey = csr
			.public_key()
			.map_err(|e| Status::internal(format!("Failed to get public key: {}", e)))?;

		let mut cert_builder = X509Builder::new()
			.map_err(|e| Status::internal(format!("Failed to create cert builder: {}", e)))?;

		cert_builder
			.set_version(2)
			.map_err(|e| Status::internal(format!("Failed to set version: {}", e)))?;
		let serial_number = {
			let mut serial =
				BigNum::new().map_err(|e| Status::internal(format!("Failed to create serial: {}", e)))?;
			serial
				.rand(159, openssl::bn::MsbOption::MAYBE_ZERO, false)
				.map_err(|e| Status::internal(format!("Failed to rand serial: {}", e)))?;
			serial
				.to_asn1_integer()
				.map_err(|e| Status::internal(format!("Failed to convert serial: {}", e)))?
		};
		cert_builder
			.set_serial_number(&serial_number)
			.map_err(|e| Status::internal(format!("Failed to set serial: {}", e)))?;
		let mut subject_name = openssl::x509::X509NameBuilder::new()
			.map_err(|e| Status::internal(format!("Failed to create subject: {}", e)))?;
		subject_name
			.append_entry_by_nid(Nid::COMMONNAME, "default")
			.map_err(|e| Status::internal(format!("Failed to set CN: {}", e)))?;
		subject_name
			.append_entry_by_nid(Nid::ORGANIZATIONNAME, "cluster.local")
			.map_err(|e| Status::internal(format!("Failed to set O: {}", e)))?;
		let subject_name = subject_name.build();
		cert_builder
			.set_subject_name(&subject_name)
			.map_err(|e| Status::internal(format!("Failed to set subject name: {}", e)))?;

		cert_builder
			.set_issuer_name(self.ca_cert.subject_name())
			.map_err(|e| Status::internal(format!("Failed to set issuer: {}", e)))?;
		let not_before = Asn1Time::days_from_now(0)
			.map_err(|e| Status::internal(format!("Failed to create not_before: {}", e)))?;
		let not_after = Asn1Time::days_from_now(365)
			.map_err(|e| Status::internal(format!("Failed to create not_after: {}", e)))?;
		cert_builder
			.set_not_before(&not_before)
			.map_err(|e| Status::internal(format!("Failed to set not_before: {}", e)))?;
		cert_builder
			.set_not_after(&not_after)
			.map_err(|e| Status::internal(format!("Failed to set not_after: {}", e)))?;

		cert_builder
			.set_pubkey(&pubkey)
			.map_err(|e| Status::internal(format!("Failed to set pubkey: {}", e)))?;
		let spiffe_id = "spiffe://cluster.local/ns/default/sa/default";
		let san = SubjectAlternativeName::new()
			.uri(spiffe_id)
			.build(&cert_builder.x509v3_context(Some(&self.ca_cert), None))
			.map_err(|e| Status::internal(format!("Failed to build SAN: {}", e)))?;
		cert_builder
			.append_extension(san)
			.map_err(|e| Status::internal(format!("Failed to add SAN: {}", e)))?;
		let key_usage = KeyUsage::new()
			.digital_signature()
			.key_encipherment()
			.build()
			.map_err(|e| Status::internal(format!("Failed to build key usage: {}", e)))?;
		cert_builder
			.append_extension(key_usage)
			.map_err(|e| Status::internal(format!("Failed to add key usage: {}", e)))?;

		cert_builder
			.sign(&self.ca_key, MessageDigest::sha256())
			.map_err(|e| Status::internal(format!("Failed to sign cert: {}", e)))?;

		let cert = cert_builder.build();
		let cert_pem = cert
			.to_pem()
			.map_err(|e| Status::internal(format!("Failed to convert cert to PEM: {}", e)))?;
		let ca_pem = self
			.ca_cert
			.to_pem()
			.map_err(|e| Status::internal(format!("Failed to convert CA to PEM: {}", e)))?;

		let cert_chain = vec![
			String::from_utf8(cert_pem)
				.map_err(|e| Status::internal(format!("Invalid UTF-8 in cert: {}", e)))?,
			String::from_utf8(ca_pem)
				.map_err(|e| Status::internal(format!("Invalid UTF-8 in CA: {}", e)))?,
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
		ca_cert: shared_ca.ca_cert.clone(),
	};

	tokio::spawn(async move {
		Server::builder()
			.add_service(IstioCertificateServiceServer::new(ca_service))
			.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
			.await
			.expect("CA server failed");
	});

	tokio::time::sleep(std::time::Duration::from_millis(100)).await;

	Ok(addr)
}
