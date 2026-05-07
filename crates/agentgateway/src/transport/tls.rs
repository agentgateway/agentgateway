use std::fmt;
use std::fmt::Formatter;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::Arc;

use agent_core::strng;
use agent_core::strng::Strng;
use futures_util::TryFutureExt;
use rustls::crypto::{CryptoProvider, SupportedKxGroup};
use rustls::server::ParsedCertificate;
use rustls::{ServerConfig, SupportedCipherSuite};
use rustls_pki_types::{CertificateDer, InvalidDnsNameError, ServerName};
use tracing::warn;
use x509_parser::certificate::X509Certificate;

use crate::apply;
use crate::serdes::schema;
use crate::transport::stream::Socket;
use crate::types::discovery::Identity;

pub static ALL_TLS_VERSIONS: &[&rustls::SupportedProtocolVersion] =
	&[&rustls::version::TLS12, &rustls::version::TLS13];

/// All currently supported cipher suites.
pub static ALL_CIPHER_SUITES: &[SupportedCipherSuite] = &[
	// TLS 1.3 cipher suites
	rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
	// TLS 1.2 cipher suites
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
];

// Default cipher suites to use if user does not specify cipher suites
pub static DEFAULT_CIPHER_SUITES: &[SupportedCipherSuite] = &[
	rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
	rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
];

pub static DEFAULT_KEY_EXCHANGE_GROUPS: &[&'static dyn SupportedKxGroup] = &[
	KeyExchangeGroup::X25519.to_supported_kx_group(),
	KeyExchangeGroup::P256.to_supported_kx_group(),
	KeyExchangeGroup::P384.to_supported_kx_group(),
	KeyExchangeGroup::X25519_MLKEM768.to_supported_kx_group(),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[allow(non_camel_case_types)]
pub enum CipherSuite {
	// TLS 1.3
	#[serde(alias = "TLS13_AES_256_GCM_SHA384")]
	TLS_AES_256_GCM_SHA384,
	#[serde(alias = "TLS13_AES_128_GCM_SHA256")]
	TLS_AES_128_GCM_SHA256,
	#[serde(alias = "TLS13_CHACHA20_POLY1305_SHA256")]
	TLS_CHACHA20_POLY1305_SHA256,

	// TLS 1.2
	#[serde(alias = "ECDHE-ECDSA-AES256-GCM-SHA384")]
	TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
	#[serde(alias = "ECDHE-ECDSA-AES128-GCM-SHA256")]
	TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
	#[serde(alias = "ECDHE-ECDSA-CHACHA20-POLY1305")]
	TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
	#[serde(alias = "ECDHE-RSA-AES256-GCM-SHA384")]
	TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
	#[serde(alias = "ECDHE-RSA-AES128-GCM-SHA256")]
	TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
	#[serde(alias = "ECDHE-RSA-CHACHA20-POLY1305")]
	TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
}

impl CipherSuite {
	pub const fn as_str_name(&self) -> &'static str {
		match self {
			// TLS 1.3
			CipherSuite::TLS_AES_256_GCM_SHA384 => "TLS_AES_256_GCM_SHA384",
			CipherSuite::TLS_AES_128_GCM_SHA256 => "TLS_AES_128_GCM_SHA256",
			CipherSuite::TLS_CHACHA20_POLY1305_SHA256 => "TLS_CHACHA20_POLY1305_SHA256",

			// TLS 1.2
			CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => {
				"TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384"
			},
			CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => {
				"TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256"
			},
			CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => {
				"TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256"
			},
			CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
			CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
			CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => {
				"TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256"
			},
		}
	}

	pub const fn to_supported_cipher_suite(&self) -> SupportedCipherSuite {
		match self {
			// TLS 1.3 cipher suites
			CipherSuite::TLS_AES_256_GCM_SHA384 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384
			},
			CipherSuite::TLS_AES_128_GCM_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256
			},
			CipherSuite::TLS_CHACHA20_POLY1305_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256
			},

			// TLS 1.2 cipher suites
			CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
			},
			CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
			},
			CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
			},
			CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
			},
			CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
			},
			CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => {
				rustls::crypto::aws_lc_rs::cipher_suite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
			},
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[allow(non_camel_case_types)]
pub enum KeyExchangeGroup {
	X25519,
	#[serde(rename = "P-256")]
	P256,
	#[serde(rename = "P-384")]
	P384,
	X25519_MLKEM768,
}

impl KeyExchangeGroup {
	pub const fn as_str_name(&self) -> &'static str {
		match self {
			KeyExchangeGroup::X25519 => "X25519",
			KeyExchangeGroup::P256 => "P-256",
			KeyExchangeGroup::P384 => "P-384",
			KeyExchangeGroup::X25519_MLKEM768 => "X25519_MLKEM768",
		}
	}

	pub const fn to_supported_kx_group(&self) -> &'static dyn SupportedKxGroup {
		match self {
			KeyExchangeGroup::X25519 => rustls::crypto::aws_lc_rs::kx_group::X25519,
			KeyExchangeGroup::P256 => rustls::crypto::aws_lc_rs::kx_group::SECP256R1,
			KeyExchangeGroup::P384 => rustls::crypto::aws_lc_rs::kx_group::SECP384R1,
			KeyExchangeGroup::X25519_MLKEM768 => rustls::crypto::aws_lc_rs::kx_group::X25519MLKEM768,
		}
	}
}

pub fn provider() -> Arc<CryptoProvider> {
	provider_with_options(&[], &[])
}

/// Returns a shared rustls `KeyLog`. On release builds this always returns
/// `NoKeyLog` so TLS session secrets can never be logged in production. On
/// debug builds, honors `SSLKEYLOGFILE` (NSS keylog format) for use with
/// Wireshark/tcpdump decryption.
#[cfg(debug_assertions)]
pub fn key_log() -> Arc<dyn rustls::KeyLog> {
	static KEY_LOG: std::sync::LazyLock<Arc<dyn rustls::KeyLog>> = std::sync::LazyLock::new(|| {
		// KeyLogFile uses an internal mutex even when SSLKEYLOGFILE is unset, so
		// guard the env check to avoid any overhead when the user is not using it.
		if std::env::var("SSLKEYLOGFILE").is_ok() {
			Arc::new(rustls::KeyLogFile::new())
		} else {
			Arc::new(rustls::NoKeyLog)
		}
	});
	(*KEY_LOG).clone()
}

#[cfg(not(debug_assertions))]
pub fn key_log() -> Arc<dyn rustls::KeyLog> {
	Arc::new(rustls::NoKeyLog)
}

/// Startup warning when SSLKEYLOGFILE is honored (debug builds only).
pub fn warn_if_key_log_enabled() {
	#[cfg(debug_assertions)]
	if let Ok(path) = std::env::var("SSLKEYLOGFILE")
		&& !path.is_empty()
	{
		warn!("SSLKEYLOGFILE={path}; TLS session secrets will be written to disk (debug build only).");
	}
}

pub fn provider_with_options(
	cipher_suites: &[CipherSuite],
	key_exchange_groups: &[KeyExchangeGroup],
) -> Arc<CryptoProvider> {
	let cipher_suites = if cipher_suites.is_empty() {
		DEFAULT_CIPHER_SUITES.to_vec()
	} else {
		cipher_suites
			.iter()
			.map(CipherSuite::to_supported_cipher_suite)
			.collect()
	};

	let key_exchange_groups = if key_exchange_groups.is_empty() {
		DEFAULT_KEY_EXCHANGE_GROUPS.to_vec()
	} else {
		key_exchange_groups
			.iter()
			.map(KeyExchangeGroup::to_supported_kx_group)
			.collect()
	};

	let mut provider = rustls::crypto::aws_lc_rs::default_provider();
	// Restrict negotiation to our allowlist.
	provider.cipher_suites = cipher_suites;
	provider.kx_groups = key_exchange_groups;
	Arc::new(provider)
}

pub fn provider_with_cipher_suites(cipher_suites: &[CipherSuite]) -> Arc<CryptoProvider> {
	provider_with_options(cipher_suites, &[])
}

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

#[derive(Debug)]
pub enum ExtendedServerName {
	Native(ServerName<'static>),

	// A URI SAN
	URI(String),
}

impl TryFrom<String> for ExtendedServerName {
	type Error = InvalidDnsNameError;

	fn try_from(value: String) -> Result<Self, Self::Error> {
		if let Ok(v) = ServerName::try_from(value.clone()) {
			Ok(Self::Native(v))
		} else if value.contains("://") {
			Ok(Self::URI(value))
		} else {
			Err(InvalidDnsNameError)
		}
	}
}

impl ExtendedServerName {
	pub fn verify_server_name(
		&self,
		cert: &ParsedCertificate,
		der: &CertificateDer,
	) -> Result<(), rustls::Error> {
		match self {
			ExtendedServerName::Native(d) => rustls::client::verify_server_name(cert, d),
			ExtendedServerName::URI(want) => {
				use x509_parser::prelude::*;

				let (_, c) = X509Certificate::from_der(der)
					.map_err(|_e| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
				let names = c
					.subject_alternative_name()
					.map_err(|_e| {
						rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName)
					})?
					.map(|x| &x.value.general_names);
				names
					.into_iter()
					.flatten()
					.filter_map(|n| match n {
						GeneralName::URI(uri) => Some(uri),
						_ => None,
					})
					.find(|cert_uri| **cert_uri == want.as_str())
					.ok_or_else(|| {
						rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName)
					})
					.map(|_| ())
			},
		}
	}
}

pub mod insecure {
	use std::sync::Arc;

	use rustls::client::WebPkiServerVerifier;
	use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
	use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
	use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
	use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

	use crate::transport::tls::provider;

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

	#[derive(Debug)]
	pub struct AltHostnameVerifier {
		roots: Arc<rustls::RootCertStore>,
		alt_server_names: Box<[super::ExtendedServerName]>,
	}

	impl AltHostnameVerifier {
		pub fn new(
			roots: Arc<rustls::RootCertStore>,
			alt_server_names: Box<[super::ExtendedServerName]>,
		) -> Self {
			Self {
				roots,
				alt_server_names,
			}
		}
	}

	// A custom verifier that allows alternative server names to be accepted.
	// Build our own verifier, inspired by https://github.com/rustls/rustls/blob/ccb79947a4811412ee7dcddcd0f51ea56bccf101/rustls/src/webpki/server_verifier.rs#L239.
	impl ServerCertVerifier for AltHostnameVerifier {
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
			let cert = rustls::server::ParsedCertificate::try_from(end_entity)?;

			let algs = provider().signature_verification_algorithms;
			rustls::client::verify_server_cert_signed_by_trust_anchor(
				&cert,
				&self.roots,
				intermediates,
				now,
				algs.all,
			)?;

			if !ocsp_response.is_empty() {
				tracing::trace!("Unvalidated OCSP response: {ocsp_response:?}");
			}

			// First attempt to verify the original server name...
			let mut last_error = None;
			for option in &self.alt_server_names {
				match option.verify_server_name(&cert, end_entity) {
					Ok(_) => return Ok(ServerCertVerified::assertion()),
					Err(e) => {
						tracing::debug!("failed to verify alt hostname {option:?} ({e})",);
						last_error = Some(e)
					},
				}
			}
			Err(last_error.unwrap_or_else(|| rustls::Error::General("unexpected error".to_string())))
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

	#[derive(Debug)]
	pub struct AllowInsecureMtlsVerifier {
		base: Arc<dyn ClientCertVerifier>,
	}

	impl AllowInsecureMtlsVerifier {
		pub fn new(base: Arc<dyn ClientCertVerifier>) -> Arc<Self> {
			Arc::new(Self { base })
		}
	}

	impl ClientCertVerifier for AllowInsecureMtlsVerifier {
		fn offer_client_auth(&self) -> bool {
			true
		}

		fn client_auth_mandatory(&self) -> bool {
			false
		}

		fn root_hint_subjects(&self) -> &[DistinguishedName] {
			self.base.root_hint_subjects()
		}

		fn verify_client_cert(
			&self,
			end_entity: &CertificateDer<'_>,
			intermediates: &[CertificateDer<'_>],
			now: UnixTime,
		) -> Result<ClientCertVerified, rustls::Error> {
			match self.base.verify_client_cert(end_entity, intermediates, now) {
				Ok(verified) => Ok(verified),
				Err(err) => {
					tracing::debug!(
						"allow_insecure_mtls: accepting client cert despite verification error: {err}"
					);
					Ok(ClientCertVerified::assertion())
				},
			}
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
		allowed_trust_domains: Arc<[Strng]>,
	}

	impl TrustDomainVerifier {
		pub fn new(
			base: Arc<dyn ClientCertVerifier>,
			allowed_trust_domains: Arc<[Strng]>,
		) -> Arc<Self> {
			Arc::new(Self {
				base,
				allowed_trust_domains,
			})
		}

		fn verify_trust_domain(&self, client_cert: &CertificateDer<'_>) -> Result<(), rustls::Error> {
			use x509_parser::prelude::*;
			if self.allowed_trust_domains.is_empty() {
				// No restriction configured; rely on CA-level trust only.
				return Ok(());
			}
			let (_, c) = X509Certificate::from_der(client_cert)
				.map_err(|_e| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))?;
			let (ids, _) = super::sans(c).map_err(|_e| {
				rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
			})?;
			trace!(
				"verifying client identities {ids:?} against allowed trust domains {:?}",
				self.allowed_trust_domains
			);
			ids
				.iter()
				.find(|id| match id {
					Identity::Spiffe { trust_domain, .. } => {
						self.allowed_trust_domains.contains(trust_domain)
					},
				})
				.ok_or_else(|| {
					rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
						Arc::new(super::LocalError::Invalid(format!(
							"identity verification error: peer did not present an allowed trustdomain \
							(allowed: [{}]), got {}",
							self.allowed_trust_domains.join(", "),
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

	#[cfg(test)]
	mod tests {
		use std::time::{Duration, SystemTime};

		use rcgen::{
			CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair,
			KeyUsagePurpose, SanType, SerialNumber,
		};
		use rustls::pki_types::CertificateDer;

		use super::*;

		/// Generate a leaf cert with a SPIFFE URI SAN for the given trust domain, signed by a test CA.
		fn make_spiffe_cert(trust_domain: &str) -> CertificateDer<'static> {
			let kp = KeyPair::generate().unwrap();
			let ca_kp = KeyPair::generate().unwrap();

			let mut params = CertificateParams::default();
			params.not_before = SystemTime::now().into();
			params.not_after = (SystemTime::now() + Duration::from_secs(3600)).into();
			params.serial_number = Some(SerialNumber::from_slice(&[1]));
			let mut dn = DistinguishedName::new();
			dn.push(DnType::OrganizationName, trust_domain);
			params.distinguished_name = dn;
			params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
			params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
			let spiffe_uri = format!("spiffe://{trust_domain}/ns/default/sa/test");
			params.subject_alt_names = vec![SanType::URI(spiffe_uri.try_into().unwrap())];

			let mut ca_params = CertificateParams::default();
			ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
			ca_params.not_before = SystemTime::now().into();
			ca_params.not_after = (SystemTime::now() + Duration::from_secs(3600)).into();
			let _ca_cert = ca_params.self_signed(&ca_kp).unwrap();
			let issuer = Issuer::from_params(&ca_params, &ca_kp);

			let cert = params.signed_by(&kp, &issuer).unwrap();
			CertificateDer::from(cert.der().to_vec())
		}

		/// Minimal no-op ClientCertVerifier — only used to satisfy TrustDomainVerifier's
		/// constructor; none of its methods are called by verify_trust_domain.
		#[derive(Debug)]
		struct NopClientVerifier;

		impl ClientCertVerifier for NopClientVerifier {
			fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
				&[]
			}

			fn verify_client_cert(
				&self,
				_end_entity: &CertificateDer<'_>,
				_intermediates: &[CertificateDer<'_>],
				_now: UnixTime,
			) -> Result<ClientCertVerified, rustls::Error> {
				Ok(ClientCertVerified::assertion())
			}

			fn verify_tls12_signature(
				&self,
				_message: &[u8],
				_cert: &CertificateDer<'_>,
				_dss: &DigitallySignedStruct,
			) -> Result<HandshakeSignatureValid, rustls::Error> {
				Ok(HandshakeSignatureValid::assertion())
			}

			fn verify_tls13_signature(
				&self,
				_message: &[u8],
				_cert: &CertificateDer<'_>,
				_dss: &DigitallySignedStruct,
			) -> Result<HandshakeSignatureValid, rustls::Error> {
				Ok(HandshakeSignatureValid::assertion())
			}

			fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
				vec![]
			}
		}

		fn verifier(domains: &[&str]) -> Arc<TrustDomainVerifier> {
			let allowed: Arc<[Strng]> = domains.iter().map(strng::new).collect();
			TrustDomainVerifier::new(Arc::new(NopClientVerifier), allowed)
		}

		#[test]
		fn empty_list_always_passes() {
			let v = verifier(&[]);
			let cert = make_spiffe_cert("any.domain");
			assert!(v.verify_trust_domain(&cert).is_ok());
		}

		#[test]
		fn single_domain_matching_cert_passes() {
			let v = verifier(&["cluster.local"]);
			let cert = make_spiffe_cert("cluster.local");
			assert!(v.verify_trust_domain(&cert).is_ok());
		}

		#[test]
		fn single_domain_mismatched_cert_rejected() {
			let v = verifier(&["cluster.local"]);
			let cert = make_spiffe_cert("other.domain");
			assert!(v.verify_trust_domain(&cert).is_err());
		}

		#[test]
		fn multiple_domains_first_matches() {
			let v = verifier(&["cluster.local", "peer.cluster"]);
			let cert = make_spiffe_cert("cluster.local");
			assert!(v.verify_trust_domain(&cert).is_ok());
		}

		#[test]
		fn multiple_domains_second_matches() {
			let v = verifier(&["cluster.local", "peer.cluster"]);
			let cert = make_spiffe_cert("peer.cluster");
			assert!(v.verify_trust_domain(&cert).is_ok());
		}

		#[test]
		fn multiple_domains_no_match_rejected() {
			let v = verifier(&["cluster.local", "peer.cluster"]);
			let cert = make_spiffe_cert("untrusted.domain");
			assert!(v.verify_trust_domain(&cert).is_err());
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
			let (id, _) = super::sans(c).map_err(|_e| {
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

#[apply(schema!)]
#[derive(cel::DynamicType, Default, Eq, PartialEq)]
pub struct TlsInfo {
	/// The (Istio SPIFFE) identity of the downstream connection, if available.
	#[serde(default)]
	pub identity: Option<IstioIdentity>,
	/// The subject alt names from the downstream certificate, if available.
	#[serde(default)]
	pub subject_alt_names: Vec<Strng>,
	/// The issuer from the downstream certificate, if available.
	#[serde(default)]
	pub issuer: Strng,
	/// The subject from the downstream certificate, if available.
	#[serde(default)]
	pub subject: Strng,
	/// The CN of the subject from the downstream certificate, if available.
	#[serde(default)]
	pub subject_cn: Option<Strng>,
}

#[apply(schema!)]
#[derive(cel::DynamicType, Eq, PartialEq)]
pub struct IstioIdentity {
	/// The trust domain of the identity.
	trust_domain: Strng,
	/// The namespace of the identity.
	namespace: Strng,
	/// The service account of the identity.
	service_account: Strng,
}

impl IstioIdentity {
	/// Create a new IstioIdentity from SPIFFE URI components.
	pub fn new(trust_domain: Strng, namespace: Strng, service_account: Strng) -> Self {
		Self {
			trust_domain,
			namespace,
			service_account,
		}
	}
}

impl fmt::Display for IstioIdentity {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"spiffe://{}/ns/{}/sa/{}",
			self.trust_domain, self.namespace, self.service_account
		)
	}
}

pub fn identity_from_connection(conn: &rustls::CommonState) -> Option<TlsInfo> {
	use x509_parser::prelude::*;
	let cert = conn
		.peer_certificates()
		.and_then(|certs| certs.first())
		.and_then(|cert| match X509Certificate::from_der(cert) {
			Ok((_, a)) => Some(a),
			Err(e) => {
				warn!("invalid certificate: {e}");
				None
			},
		})?;

	let (issuer, subject, subject_cn) = names(&cert);
	let (istio, sans) = sans(cert).ok()?;
	Some(TlsInfo {
		identity: istio.into_iter().next().map(|i| {
			let Identity::Spiffe {
				trust_domain,
				namespace,
				service_account,
			} = i;
			IstioIdentity {
				trust_domain,
				namespace,
				service_account,
			}
		}),
		subject_alt_names: sans,
		issuer,
		subject,
		subject_cn,
	})
}
fn names(cert: &X509Certificate) -> (Strng, Strng, Option<Strng>) {
	let issuer = cert.issuer().to_string().into();
	let subject = cert.subject().to_string().into();
	let subject_cn = cert
		.subject
		.iter_common_name()
		.find_map(|x| x.as_str().ok())
		.map(strng::new);
	(issuer, subject, subject_cn)
}
fn sans(cert: X509Certificate) -> anyhow::Result<(Vec<Identity>, Vec<Strng>)> {
	use x509_parser::prelude::*;
	let names = cert
		.subject_alternative_name()?
		.map(|x| &x.value.general_names);

	if let Some(names) = names {
		let istio = names
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
			.collect();
		let generic = names
			.iter()
			.filter_map(|n| match n {
				GeneralName::URI(uri) => Some(strng::new(uri)),
				GeneralName::DNSName(n) => Some(strng::new(n)),
				GeneralName::IPAddress(ip) => match ip.len() {
					4 => {
						let array: [u8; 4] = (*ip).try_into().unwrap();
						Some(strng::new(IpAddr::V4(Ipv4Addr::from(array)).to_string()))
					},
					16 => {
						let array: [u8; 16] = (*ip).try_into().unwrap();
						Some(strng::new(IpAddr::V6(Ipv6Addr::from(array)).to_string()))
					},
					_ => None,
				},
				_ => None,
			})
			.collect();
		return Ok((istio, generic));
	}
	Ok((Vec::default(), Vec::default()))
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
