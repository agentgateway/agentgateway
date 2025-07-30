use std::io;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use once_cell::sync::Lazy;
use rustls::ClientConfig;
use serde::Serializer;

use crate::transport::tls;
use crate::types::agent::{parse_cert, parse_key};
use crate::{serdes, transport};

pub static SYSTEM_TRUST: Lazy<BackendTLS> =
	Lazy::new(|| LocalBackendTLS::default().try_into().unwrap());
pub static INSECURE_TRUST: Lazy<BackendTLS> = Lazy::new(|| {
	LocalBackendTLS {
		cert: None,
		key: None,
		root: None,
		insecure: true,
		insecure_host: false,
	}
	.try_into()
	.unwrap()
});

// TODO: xds support
#[derive(Debug, Clone)]
pub struct BackendTLS {
	pub config: Arc<ClientConfig>,
}

impl std::hash::Hash for BackendTLS {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		// Hash the pointer address
		Arc::as_ptr(&self.config).hash(state);
	}
}

impl PartialEq for BackendTLS {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.config, &other.config)
	}
}
impl Eq for BackendTLS {}

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
	#[serde(default)]
	insecure: bool,
	#[serde(default)]
	insecure_host: bool,
}
pub struct ResolvedBackendTLS {
	pub cert: Option<Vec<u8>>,
	pub key: Option<Vec<u8>>,
	pub root: Option<Vec<u8>>,
	pub insecure: bool,
	pub insecure_host: bool,
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
		let mut ccb = ClientConfig::builder_with_provider(transport::tls::provider())
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
		}
		// TODO: support configuring
		cc.alpn_protocols = vec![b"h2".into(), b"http/1.1".into()];
		// cc.alpn_protocols = vec![b"http/1.1".into()];
		Ok(BackendTLS {
			config: Arc::new(cc),
		})
	}
}

impl LocalBackendTLS {
	pub fn try_into(self) -> anyhow::Result<BackendTLS> {
		ResolvedBackendTLS {
			cert: self.cert.map(fs_err::read).transpose()?,
			key: self.key.map(fs_err::read).transpose()?,
			root: self.root.map(fs_err::read).transpose()?,
			insecure: self.insecure,
			insecure_host: self.insecure_host,
		}
		.try_into()
	}
}
