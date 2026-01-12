use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use once_cell::sync::Lazy;
use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use serde::Serializer;

use crate::transport;
use crate::transport::tls;
use crate::types::agent::{parse_cert, parse_key};

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
	}
	.try_into()
	.unwrap()
});

// a ClientConfig stores the ALPN, but we need to set it per request possibly. This struct helps manage that.
#[derive(Clone, Debug)]
pub struct PerAlpnConfig {
	config: Arc<ClientConfig>,
	allow_custom_alpn: bool,
	h1: std::sync::OnceLock<Arc<ClientConfig>>,
	h2: std::sync::OnceLock<Arc<ClientConfig>>,
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
		// TODO: store raw pem so we can send it back
		serializer.serialize_none()
	}
}
static SYSTEM_ROOT: Lazy<rustls_native_certs::CertificateResult> =
	Lazy::new(rustls_native_certs::load_native_certs);

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocalBackendTLS {
	cert: Option<PathBuf>,
	key: Option<PathBuf>,
	root: Option<PathBuf>,
	// If set, override the SNI. Otherwise, it will automatically be set.
	hostname: Option<String>,
	#[serde(default)]
	insecure: bool,
	#[serde(default)]
	insecure_host: bool,
	#[serde(default)]
	alpn: Option<Vec<String>>,
	#[serde(default)]
	pub subject_alt_names: Option<Vec<String>>,
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
}

impl ResolvedBackendTLS {
	pub fn try_into(self) -> anyhow::Result<BackendTLS> {
		let mut roots = rustls::RootCertStore::empty();
		if let Some(root) = self.root {
			let mut reader = std::io::BufReader::new(Cursor::new(root));
			let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
			roots.add_parsable_certificates(certs);
		} else {
			// TODO: we probably should do this once globally!
			for cert in &crate::http::backendtls::SYSTEM_ROOT.certs {
				roots.add(cert.clone()).unwrap();
			}
		}

		let roots = Arc::new(roots);
		let ccb = ClientConfig::builder_with_provider(transport::tls::provider())
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
			let inner = rustls::client::WebPkiServerVerifier::builder_with_provider(
				roots,
				transport::tls::provider(),
			)
			.build()?;
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
		let allow_custom_alpn = self.alpn.is_none();
		if let Some(a) = self.alpn {
			cc.alpn_protocols = a.into_iter().map(|b| b.as_bytes().to_vec()).collect();
		} else {
			cc.alpn_protocols = vec![b"h2".into(), b"http/1.1".into()];
		}
		Ok(BackendTLS {
			hostname_override: self.hostname.map(|s| s.try_into()).transpose()?,
			config: PerAlpnConfig::new(Arc::new(cc), allow_custom_alpn),
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
		}
		.try_into()
	}
}
