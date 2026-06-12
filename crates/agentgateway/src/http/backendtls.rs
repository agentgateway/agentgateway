use std::path::PathBuf;
use std::sync::Arc;

use agent_core::strng;
use agent_core::strng::Strng;
use once_cell::sync::Lazy;
use rustls::ClientConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use serde::Serializer;
use tracing::trace;

use crate::serdes::schema_ser;
use crate::transport::tls;
use crate::types::agent::{parse_cert, parse_key};
use crate::{apply, transport};

pub static SYSTEM_TRUST: Lazy<BackendTLS> =
	Lazy::new(|| LocalBackendTLS::default().try_into().unwrap());
pub static INSECURE_TRUST: Lazy<BackendTLS> = Lazy::new(|| {
	LocalBackendTLS {
		cert: None,
		key: None,
		root: None,
		hostname: None,
		insecure: true,
		insecure_host: false,
		alpn: None,
		subject_alt_names: None,
		key_exchange_groups: None,
	}
	.try_into()
	.unwrap()
});

// a ClientConfig stores the ALPN, but we need to set it per request possibly. This struct helps manage that.
#[derive(Clone, Debug)]
pub struct PerAlpnConfig {
	config: Arc<ClientConfig>,
	allow_custom_alpn: bool,
	h1: Arc<std::sync::OnceLock<Arc<ClientConfig>>>,
	h2: Arc<std::sync::OnceLock<Arc<ClientConfig>>>,
}

impl PerAlpnConfig {
	pub fn new(config: Arc<ClientConfig>, allow_custom_alpn: bool) -> Self {
		Self {
			config,
			allow_custom_alpn,
			h1: Default::default(),
			h2: Default::default(),
		}
	}

	fn config_for(&self, version_override: Option<http::Version>) -> Arc<ClientConfig> {
		match version_override {
			Some(http::Version::HTTP_11) if self.allow_custom_alpn => self
				.h1
				.get_or_init(|| {
					let mut nc = Arc::unwrap_or_clone(self.config.clone());
					nc.alpn_protocols = vec![b"http/1.1".to_vec()];
					Arc::new(nc)
				})
				.clone(),
			Some(http::Version::HTTP_2) if self.allow_custom_alpn => self
				.h2
				.get_or_init(|| {
					let mut nc = Arc::unwrap_or_clone(self.config.clone());
					nc.alpn_protocols = vec![b"h2".to_vec()];
					Arc::new(nc)
				})
				.clone(),
			_ => self.config.clone(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct BackendTLS {
	pub hostname_override: Option<ServerName<'static>>,
	pub config: PerAlpnConfig,
	pub metadata: BackendTLSInfo,
}

impl BackendTLS {
	pub fn base_config(&self) -> VersionedBackendTLS {
		self.config_for(None)
	}

	pub fn config_for(&self, version_override: Option<http::Version>) -> VersionedBackendTLS {
		VersionedBackendTLS {
			hostname_override: self.hostname_override.clone(),
			config: self.config.config_for(version_override),
		}
	}
}

#[derive(Debug, Clone)]
pub struct VersionedBackendTLS {
	pub hostname_override: Option<ServerName<'static>>,
	pub config: Arc<ClientConfig>,
}

impl std::hash::Hash for VersionedBackendTLS {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		// Hash the pointer address
		Arc::as_ptr(&self.config).hash(state);
		self.hostname_override.hash(state);
	}
}

impl PartialEq for VersionedBackendTLS {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.config, &other.config) && self.hostname_override == other.hostname_override
	}
}

impl Eq for VersionedBackendTLS {}

impl serde::Serialize for BackendTLS {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serde::Serialize::serialize(&self.metadata, serializer)
	}
}

#[apply(schema_ser!)]
#[derive(Default)]
pub struct BackendTLSInfo {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cert: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub root: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub hostname: Option<String>,
	#[serde(default, skip_serializing_if = "is_false")]
	pub insecure: bool,
	#[serde(default, skip_serializing_if = "is_false")]
	pub insecure_host: bool,
	#[serde(default, skip_serializing_if = "is_false")]
	pub system_roots: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub alpn: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subject_alt_names: Option<Vec<String>>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub key_exchange_groups: Option<Vec<tls::KeyExchangeGroup>>,
}

impl BackendTLSInfo {
	pub fn from_resolved(tls: &ResolvedBackendTLS) -> Self {
		Self {
			cert: tls.cert.as_ref().map(pem_to_string),
			root: tls.root.as_ref().map(pem_to_string),
			hostname: tls.hostname.clone(),
			insecure: tls.insecure,
			insecure_host: tls.insecure_host,
			system_roots: tls.root.is_none(),
			alpn: tls.alpn.clone(),
			subject_alt_names: tls.subject_alt_names.clone(),
			key_exchange_groups: tls.key_exchange_groups.clone(),
		}
	}
}

fn pem_to_string(pem: impl AsRef<[u8]>) -> Strng {
	strng::new(String::from_utf8_lossy(pem.as_ref()))
}

fn is_false(value: &bool) -> bool {
	!*value
}
static SYSTEM_ROOT: Lazy<rustls_native_certs::CertificateResult> =
	Lazy::new(rustls_native_certs::load_native_certs);

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocalBackendTLS {
	/// Client certificate file to present to the backend.
	cert: Option<PathBuf>,
	/// Private key file for the client certificate.
	key: Option<PathBuf>,
	/// Root certificate bundle used to verify the backend certificate.
	root: Option<PathBuf>,
	/// Server name to use for TLS verification and SNI.
	hostname: Option<String>,
	/// Skip certificate trust verification for the backend connection.
	#[serde(default)]
	insecure: bool,
	/// Skip hostname verification for the backend certificate.
	#[serde(default)]
	insecure_host: bool,
	/// ALPN protocols to offer to the backend.
	#[serde(default)]
	alpn: Option<Vec<String>>,
	/// Additional subject alternative names accepted for the backend certificate.
	#[serde(default)]
	pub subject_alt_names: Option<Vec<String>>,
	/// Key exchange groups allowed for negotiating TLS.
	#[serde(default)]
	key_exchange_groups: Option<Vec<tls::KeyExchangeGroup>>,
}

#[derive(Default, Debug)]
pub struct ResolvedBackendTLS {
	pub cert: Option<Vec<u8>>,
	pub key: Option<Vec<u8>>,
	pub root: Option<Vec<u8>>,
	// If set, override the SNI. Otherwise, it will automatically be set.
	pub hostname: Option<String>,
	pub insecure: bool,
	pub insecure_host: bool,
	pub alpn: Option<Vec<String>>,
	pub subject_alt_names: Option<Vec<String>>,
	pub key_exchange_groups: Option<Vec<tls::KeyExchangeGroup>>,
}

impl ResolvedBackendTLS {
	pub fn try_into(self) -> anyhow::Result<BackendTLS> {
		let metadata = BackendTLSInfo::from_resolved(&self);
		let mut roots = rustls::RootCertStore::empty();
		if let Some(root) = self.root {
			let certs = CertificateDer::pem_slice_iter(&root).collect::<Result<Vec<_>, _>>()?;
			let (valid, invalid) = roots.add_parsable_certificates(certs);
			trace!(valid, invalid, "added root certificates")
		} else {
			// TODO: we probably should do this once globally!
			for cert in &crate::http::backendtls::SYSTEM_ROOT.certs {
				roots.add(cert.clone()).unwrap();
			}
		}

		let roots = Arc::new(roots);
		let provider = transport::tls::provider_with_options(
			&[],
			self.key_exchange_groups.as_deref().unwrap_or_default(),
		);
		let ccb = ClientConfig::builder_with_provider(provider.clone())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid")
			.with_root_certificates(roots.clone());

		let mut cc = match (self.cert, self.key) {
			(Some(cert), Some(key)) => {
				let cert_chain = parse_cert(&cert)?;
				let private_key = parse_key(&key)?;
				ccb.with_client_auth_cert(cert_chain, private_key)?
			},
			_ => ccb.with_no_client_auth(),
		};
		if self.insecure_host {
			let inner =
				rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider).build()?;
			let verifier = Arc::new(tls::insecure::NoServerNameVerification::new(inner));
			cc.dangerous().set_certificate_verifier(verifier);
		} else if self.insecure {
			cc.dangerous()
				.set_certificate_verifier(Arc::new(tls::insecure::NoVerifier));
		} else if let Some(alt_sans) = self.subject_alt_names {
			let sans = alt_sans
				.into_iter()
				.map(tls::ExtendedServerName::try_from)
				.collect::<Result<Box<_>, _>>()?;
			cc.dangerous()
				.set_certificate_verifier(Arc::new(tls::insecure::AltHostnameVerifier::new(
					roots, sans,
				)));
		}
		cc.key_log = transport::tls::key_log();
		let allow_custom_alpn = self.alpn.is_none();
		if let Some(a) = self.alpn {
			cc.alpn_protocols = a.into_iter().map(|b| b.as_bytes().to_vec()).collect();
		} else {
			cc.alpn_protocols = vec![b"h2".into(), b"http/1.1".into()];
		}
		Ok(BackendTLS {
			hostname_override: self.hostname.map(|s| s.try_into()).transpose()?,
			config: PerAlpnConfig::new(Arc::new(cc), allow_custom_alpn),
			metadata,
		})
	}
}

impl LocalBackendTLS {
	pub fn try_into(self) -> anyhow::Result<BackendTLS> {
		ResolvedBackendTLS {
			cert: self.cert.map(fs_err::read).transpose()?,
			key: self.key.map(fs_err::read).transpose()?,
			root: self.root.map(fs_err::read).transpose()?,
			hostname: self.hostname,
			insecure: self.insecure,
			insecure_host: self.insecure_host,
			alpn: self.alpn,
			subject_alt_names: self.subject_alt_names,
			key_exchange_groups: self.key_exchange_groups,
		}
		.try_into()
	}
}
