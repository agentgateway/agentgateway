use std::str::FromStr;
use std::sync::Arc;

use futures_util::TryFutureExt;
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use tracing::warn;
use x509_parser::certificate::X509Certificate;

use crate::transport::stream::Socket;
use crate::types::discovery::Identity;

pub static ALL_TLS_VERSIONS: &[&rustls::SupportedProtocolVersion] =
	&[&rustls::version::TLS12, &rustls::version::TLS13];

pub fn provider() -> Arc<CryptoProvider> {
	Arc::new(CryptoProvider {
		// Limit to only the subset of ciphers that are FIPS compatible
		cipher_suites: vec![
			// TLS 1.3 cipher suites
			rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
			rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
			// TLS 1.2 cipher suites
			rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
			rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
			rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
			rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
		],
		..rustls::crypto::aws_lc_rs::default_provider()
	})
}

// pub fn provider() -> Arc<CryptoProvider> {
// 	Arc::new(CryptoProvider {
// 		// Limit to only the subset of ciphers that are FIPS compatible
// 		cipher_suites: vec![
// 			rustls::crypto::ring::cipher_suite::TLS13_AES_256_GCM_SHA384,
// 			rustls::crypto::ring::cipher_suite::TLS13_AES_128_GCM_SHA256,
// 		],
// 		..rustls::crypto::ring::default_provider()
// 	})
// }

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("tls handshake error: {0:?}")]
	Handshake(std::io::Error),
	#[error("{0}")]
	Anyhow(#[from] anyhow::Error),
}

pub async fn accept(conn: Socket, cfg: Arc<ServerConfig>) -> Result<Socket, Error> {
	let (ext, counter, inner) = conn.into_parts();
	let tls_cfg = cfg.clone();
	let stream = tokio_rustls::TlsAcceptor::from(tls_cfg)
		.accept(Box::new(inner))
		.map_err(Error::Handshake)
		.await?;
	Ok(Socket::from_tls(ext, counter, stream.into())?)
}

pub mod insecure {
	use std::sync::Arc;

	use rustls::client::WebPkiServerVerifier;
	use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
	use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
	use rustls::{DigitallySignedStruct, SignatureScheme};

	#[derive(Debug)]
	pub struct NoServerNameVerification {
		inner: Arc<WebPkiServerVerifier>,
	}

	impl NoServerNameVerification {
		pub fn new(inner: Arc<WebPkiServerVerifier>) -> Self {
			Self { inner }
		}
	}

	impl ServerCertVerifier for NoServerNameVerification {
		fn verify_server_cert(
			&self,
			_end_entity: &CertificateDer<'_>,
			_intermediates: &[CertificateDer<'_>],
			_server_name: &ServerName<'_>,
			_ocsp: &[u8],
			_now: UnixTime,
		) -> Result<ServerCertVerified, rustls::Error> {
			match self
				.inner
				.verify_server_cert(_end_entity, _intermediates, _server_name, _ocsp, _now)
			{
				Ok(scv) => Ok(scv),
				Err(rustls::Error::InvalidCertificate(cert_error)) => {
					if matches!(
						cert_error,
						rustls::CertificateError::NotValidForName
							| rustls::CertificateError::NotValidForNameContext { .. }
					) {
						Ok(ServerCertVerified::assertion())
					} else {
						Err(rustls::Error::InvalidCertificate(cert_error))
					}
				},
				Err(e) => Err(e),
			}
		}

		fn verify_tls12_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			self.inner.verify_tls12_signature(message, cert, dss)
		}

		fn verify_tls13_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			self.inner.verify_tls13_signature(message, cert, dss)
		}

		fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
			self.inner.supported_verify_schemes()
		}
	}

	#[derive(Debug)]
	pub struct NoVerifier;

	impl ServerCertVerifier for NoVerifier {
		fn verify_server_cert(
			&self,
			_end_entity: &rustls_pki_types::CertificateDer,
			_intermediates: &[rustls_pki_types::CertificateDer],
			_server_name: &ServerName,
			_ocsp_response: &[u8],
			_now: UnixTime,
		) -> Result<ServerCertVerified, rustls::Error> {
			Ok(ServerCertVerified::assertion())
		}

		fn verify_tls12_signature(
			&self,
			_message: &[u8],
			_cert: &rustls_pki_types::CertificateDer,
			_dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			Ok(HandshakeSignatureValid::assertion())
		}

		fn verify_tls13_signature(
			&self,
			_message: &[u8],
			_cert: &rustls_pki_types::CertificateDer,
			_dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			Ok(HandshakeSignatureValid::assertion())
		}

		fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
			vec![
				SignatureScheme::RSA_PKCS1_SHA1,
				SignatureScheme::ECDSA_SHA1_Legacy,
				SignatureScheme::RSA_PKCS1_SHA256,
				SignatureScheme::ECDSA_NISTP256_SHA256,
				SignatureScheme::RSA_PKCS1_SHA384,
				SignatureScheme::ECDSA_NISTP384_SHA384,
				SignatureScheme::RSA_PKCS1_SHA512,
				SignatureScheme::ECDSA_NISTP521_SHA512,
				SignatureScheme::RSA_PSS_SHA256,
				SignatureScheme::RSA_PSS_SHA384,
				SignatureScheme::RSA_PSS_SHA512,
				SignatureScheme::ED25519,
				SignatureScheme::ED448,
			]
		}
	}
}

pub mod trustdomain {

	use std::fmt::Debug;
	use std::sync::Arc;

	use rustls::client::danger::HandshakeSignatureValid;
	use rustls::pki_types::{CertificateDer, UnixTime};
	use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
	use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

	use crate::types::discovery::Identity;
	use crate::*;

	#[derive(Debug)]
	pub struct TrustDomainVerifier {
		base: Arc<dyn ClientCertVerifier>,
		trust_domain: Option<Strng>,
	}

	impl TrustDomainVerifier {
		pub fn new(base: Arc<dyn ClientCertVerifier>, trust_domain: Option<Strng>) -> Arc<Self> {
			Arc::new(Self { base, trust_domain })
		}

		fn verify_trust_domain(&self, client_cert: &CertificateDer<'_>) -> Result<(), rustls::Error> {
			use x509_parser::prelude::*;
			let Some(want_trust_domain) = &self.trust_domain else {
				// No need to verify
				return Ok(());
			};
			let (_, c) = X509Certificate::from_der(client_cert)
				.map_err(|_e| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
			let ids = super::identities(c).map_err(|_e| {
				rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
			})?;
			trace!(
				"verifying client identities {ids:?} against trust domain {:?}",
				want_trust_domain
			);
			ids
				.iter()
				.find(|id| match id {
					Identity::Spiffe { trust_domain, .. } => trust_domain == want_trust_domain,
				})
				.ok_or_else(|| {
					rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
						Arc::new(super::LocalError::Invalid(format!(
							"identity verification error: peer did not present the expected trustdomain ({}), got {}",
							&self.trust_domain.as_ref().unwrap(),
							super::display_list(&ids)
						))),
					)))
				})
				.map(|_| ())
		}
	}

	// Implement our custom ClientCertVerifier logic. We only want to add an extra check, but
	// need a decent amount of boilerplate to do so.
	impl ClientCertVerifier for TrustDomainVerifier {
		fn root_hint_subjects(&self) -> &[DistinguishedName] {
			self.base.root_hint_subjects()
		}

		fn verify_client_cert(
			&self,
			end_entity: &CertificateDer<'_>,
			intermediates: &[CertificateDer<'_>],
			now: UnixTime,
		) -> Result<ClientCertVerified, rustls::Error> {
			let res = self
				.base
				.verify_client_cert(end_entity, intermediates, now)?;
			self.verify_trust_domain(end_entity)?;
			Ok(res)
		}

		fn verify_tls12_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			self.base.verify_tls12_signature(message, cert, dss)
		}

		fn verify_tls13_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			self.base.verify_tls13_signature(message, cert, dss)
		}

		fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
			self.base.supported_verify_schemes()
		}
	}
}

pub mod identity {

	use std::fmt::Debug;
	use std::sync::Arc;

	use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
	use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
	use rustls::server::ParsedCertificate;
	use rustls::{DigitallySignedStruct, SignatureScheme};
	use tracing::debug;

	use crate::transport::tls::provider;
	use crate::types::discovery::Identity;
	use crate::*;

	#[derive(Debug)]
	pub struct IdentityVerifier {
		pub roots: Arc<rustls::RootCertStore>,
		pub identity: Vec<Identity>,
	}

	impl IdentityVerifier {
		fn verify_full_san(&self, server_cert: &CertificateDer<'_>) -> Result<(), rustls::Error> {
			use x509_parser::prelude::*;
			let (_, c) = X509Certificate::from_der(server_cert)
				.map_err(|_e| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
			let id = super::identities(c).map_err(|_e| {
				rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
			})?;
			trace!(
				"verifying server identities {id:?} against {:?}",
				self.identity
			);
			for ident in id.iter() {
				if let Some(_i) = self.identity.iter().find(|id| id == &ident) {
					return Ok(());
				}
			}
			debug!("identity mismatch {id:?} != {:?}", self.identity);
			Err(rustls::Error::InvalidCertificate(
				rustls::CertificateError::Other(rustls::OtherError(Arc::new(super::LocalError::Invalid(
					format!(
						"identity verification error: peer did not present the expected trustdomain ({}), got {}",
						super::display_list(&self.identity),
						super::display_list(&id)
					),
				)))),
			))
		}
	}

	// Rustls doesn't natively validate URI SAN.
	// Build our own verifier, inspired by https://github.com/rustls/rustls/blob/ccb79947a4811412ee7dcddcd0f51ea56bccf101/rustls/src/webpki/server_verifier.rs#L239.
	impl ServerCertVerifier for IdentityVerifier {
		/// Will verify the certificate is valid in the following ways:
		/// - Signed by a  trusted `RootCertStore` CA
		/// - Not Expired
		fn verify_server_cert(
			&self,
			end_entity: &CertificateDer<'_>,
			intermediates: &[CertificateDer<'_>],
			_sn: &ServerName,
			ocsp_response: &[u8],
			now: UnixTime,
		) -> Result<ServerCertVerified, rustls::Error> {
			let cert = ParsedCertificate::try_from(end_entity)?;

			let algs = provider().signature_verification_algorithms;
			rustls::client::verify_server_cert_signed_by_trust_anchor(
				&cert,
				&self.roots,
				intermediates,
				now,
				algs.all,
			)?;

			if !ocsp_response.is_empty() {
				trace!("Unvalidated OCSP response: {ocsp_response:?}");
			}

			self.verify_full_san(end_entity)?;

			Ok(ServerCertVerified::assertion())
		}

		// Rest use the default implementations

		fn verify_tls12_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls12_signature(
				message,
				cert,
				dss,
				&provider().signature_verification_algorithms,
			)
		}

		fn verify_tls13_signature(
			&self,
			message: &[u8],
			cert: &CertificateDer<'_>,
			dss: &DigitallySignedStruct,
		) -> Result<HandshakeSignatureValid, rustls::Error> {
			rustls::crypto::verify_tls13_signature(
				message,
				cert,
				dss,
				&provider().signature_verification_algorithms,
			)
		}

		fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
			provider()
				.signature_verification_algorithms
				.supported_schemes()
		}
	}
}

pub fn identity_from_connection(conn: &rustls::CommonState) -> Option<Identity> {
	use x509_parser::prelude::*;
	conn
		.peer_certificates()
		.and_then(|certs| certs.first())
		.and_then(|cert| match X509Certificate::from_der(cert) {
			Ok((_, a)) => Some(a),
			Err(e) => {
				warn!("invalid certificate: {e}");
				None
			},
		})
		.and_then(|cert| match identities(cert) {
			Ok(ids) => ids.into_iter().next(),
			Err(e) => {
				warn!("failed to extract identity: {}", e);
				None
			},
		})
}

fn identities(cert: X509Certificate) -> anyhow::Result<Vec<Identity>> {
	use x509_parser::prelude::*;
	let names = cert
		.subject_alternative_name()?
		.map(|x| &x.value.general_names);

	if let Some(names) = names {
		return Ok(
			names
				.iter()
				.filter_map(|n| {
					let id = match n {
						GeneralName::URI(uri) => Identity::from_str(uri),
						_ => return None,
					};

					match id {
						Ok(id) => Some(id),
						Err(err) => {
							warn!("SAN {n} could not be parsed: {err}");
							None
						},
					}
				})
				.collect(),
		);
	}
	Ok(Vec::default())
}

#[derive(thiserror::Error, Debug)]
enum LocalError {
	#[error("{0}")]
	Invalid(String),
}

fn display_list<T: ToString>(i: &[T]) -> String {
	i.iter()
		.map(|id| id.to_string())
		.collect::<Vec<String>>()
		.join(",")
}
