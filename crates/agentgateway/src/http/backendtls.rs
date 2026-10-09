use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use agent_core::strng;
use agent_core::strng::Strng;
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use rustls::ClientConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use serde::Serializer;
use tracing::{info, trace, warn};

use crate::serdes::{schema_de, schema_ser};
use crate::transport::tls;
use crate::types::agent::{parse_cert, parse_key};
use crate::{apply, transport};

pub static SYSTEM_TRUST: Lazy<BackendTLS> =
	Lazy::new(|| ResolvedBackendTLS::default().try_into().unwrap());
#[derive(Debug)]
struct PerAlpnConfigData {
	config: Arc<ClientConfig>,
	allow_custom_alpn: bool,
	h1: OnceLock<Arc<ClientConfig>>,
	h2: OnceLock<Arc<ClientConfig>>,
}

impl PerAlpnConfigData {
	fn new(config: Arc<ClientConfig>, allow_custom_alpn: bool) -> Self {
		Self {
			config,
			allow_custom_alpn,
			h1: OnceLock::new(),
			h2: OnceLock::new(),
		}
	}
}

// a ClientConfig stores the ALPN, but we need to set it per request possibly. This struct helps manage that.
// The ArcSwap lets file-sourced certs hot-reload without a new xDS push.
#[derive(Clone, Debug)]
pub struct PerAlpnConfig {
	data: Arc<ArcSwap<PerAlpnConfigData>>,
	/// Aborts the cert-file watcher once every clone of this config is dropped.
	watcher: Option<Arc<WatcherHandle>>,
}

#[derive(Debug)]
struct WatcherHandle(tokio::task::JoinHandle<()>);

impl Drop for WatcherHandle {
	fn drop(&mut self) {
		self.0.abort();
	}
}

fn store_into(
	data: &ArcSwap<PerAlpnConfigData>,
	config: Arc<ClientConfig>,
	allow_custom_alpn: bool,
) {
	data.store(Arc::new(PerAlpnConfigData::new(config, allow_custom_alpn)));
}

impl PerAlpnConfig {
	pub fn new(config: Arc<ClientConfig>, allow_custom_alpn: bool) -> Self {
		Self {
			data: Arc::new(ArcSwap::new(Arc::new(PerAlpnConfigData::new(
				config,
				allow_custom_alpn,
			)))),
			watcher: None,
		}
	}

	/// Held by the watcher instead of a `PerAlpnConfig`, which would keep its own abort guard alive.
	fn data_handle(&self) -> Arc<ArcSwap<PerAlpnConfigData>> {
		self.data.clone()
	}

	fn with_watcher(mut self, task: tokio::task::JoinHandle<()>) -> Self {
		self.watcher = Some(Arc::new(WatcherHandle(task)));
		self
	}

	#[cfg(test)]
	fn watcher_weak(&self) -> Option<std::sync::Weak<WatcherHandle>> {
		self.watcher.as_ref().map(Arc::downgrade)
	}

	pub fn config_for(&self, version_override: Option<http::Version>) -> Arc<ClientConfig> {
		let data = self.data.load();
		match version_override {
			Some(http::Version::HTTP_11) if data.allow_custom_alpn => data
				.h1
				.get_or_init(|| {
					let mut nc = Arc::unwrap_or_clone(data.config.clone());
					nc.alpn_protocols = vec![b"http/1.1".to_vec()];
					Arc::new(nc)
				})
				.clone(),
			Some(http::Version::HTTP_2) if data.allow_custom_alpn => data
				.h2
				.get_or_init(|| {
					let mut nc = Arc::unwrap_or_clone(data.config.clone());
					nc.alpn_protocols = vec![b"h2".to_vec()];
					Arc::new(nc)
				})
				.clone(),
			_ => data.config.clone(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct BackendTLS {
	pub hostname_override: Option<ServerName<'static>>,
	pub source: BackendTLSSource,
	pub metadata: BackendTLSInfo,
}

/// Where the upstream `ClientConfig` comes from.
#[derive(Debug, Clone)]
pub enum BackendTLSSource {
	/// A fully-built config from inline cert/key/root (or system roots).
	Static(PerAlpnConfig),
	/// Sourced from the SPIFFE Workload API at connection time (SVID rotates), resolved via
	/// `SpiffeClient::client_config`. See `proxy::httpproxy::resolve_backend_tls`.
	Spiffe(SpiffeBackendTLS),
}

/// Parameters needed to build a SPIFFE-sourced upstream `ClientConfig` at connection time.
#[derive(Debug, Clone)]
pub struct SpiffeBackendTLS {
	/// Explicit ALPN protocols; `None` means the default `h2,http/1.1` (and allows a per-request
	/// HTTP-version hint to narrow the offered set).
	pub alpn: Option<Vec<String>>,
	/// Expected upstream SPIFFE IDs to pin; empty means accept any SVID chaining to the bundle.
	pub verify_sans: Vec<String>,
}

impl BackendTLS {
	/// Returns the static config for the requested HTTP version. Only valid for
	/// [`BackendTLSSource::Static`]; SPIFFE-sourced backends are resolved at connection time via
	/// `proxy::httpproxy::resolve_backend_tls` and must not reach here.
	pub fn base_config(&self) -> VersionedBackendTLS {
		let BackendTLSSource::Static(config) = &self.source else {
			panic!(
				"base_config is only valid for static backend TLS; SPIFFE backends resolve per connection"
			)
		};
		VersionedBackendTLS {
			hostname_override: self.hostname_override.clone(),
			config: config.config_for(None),
			peer_identity_mode: tls::PeerIdentityMode::Istio,
		}
	}
}

#[derive(Debug, Clone)]
pub struct VersionedBackendTLS {
	pub hostname_override: Option<ServerName<'static>>,
	pub config: Arc<ClientConfig>,
	/// How to interpret the upstream server's SPIFFE identity when extracting peer TLS info.
	/// `Spiffe` for SPIFFE-sourced backends so their non-Istio SVIDs are not parsed as Istio identities.
	pub peer_identity_mode: tls::PeerIdentityMode,
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
	#[serde(default, skip_serializing_if = "is_false")]
	pub spiffe: bool,
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
			system_roots: tls.root.is_none() && tls.root_path.is_none() && !tls.spiffe,
			spiffe: tls.spiffe,
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

#[apply(schema_de!)]
#[derive(Default)]
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
	/// Get the gateway's client identity and trust roots from the SPIFFE Workload API.
	/// Mutually exclusive with `cert`/`key`/`root`/`insecure`/`insecureHost`.
	/// Pin specific upstream SPIFFE IDs via `subjectAltNames` (e.g. `spiffe://td/ns/foo/sa/bar`);
	/// If `subjectAltNames` is omitted, any SVID chaining to the SPIFFE trust bundle is accepted
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub spiffe: Option<LocalSpiffeBackendTLS>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct LocalSpiffeBackendTLS {} // Empty config for now, allows values to be added in the future.

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
	/// Files to read `cert`/`key`/`root` from; watched and reloaded on change.
	pub cert_path: Option<PathBuf>,
	pub key_path: Option<PathBuf>,
	pub root_path: Option<PathBuf>,
	pub spiffe: bool,
}

/// Everything but the cert/key/root material, which file-sourced configs re-read on each reload.
#[derive(Clone, Default)]
struct TlsSettings {
	insecure: bool,
	insecure_host: bool,
	subject_alt_names: Option<Vec<String>>,
	key_exchange_groups: Option<Vec<tls::KeyExchangeGroup>>,
	alpn: Option<Vec<String>>,
}

#[derive(Default)]
struct CertMaterial {
	cert: Option<Vec<u8>>,
	key: Option<Vec<u8>>,
	root: Option<Vec<u8>>,
}

fn build_client_config(
	material: CertMaterial,
	settings: &TlsSettings,
) -> anyhow::Result<(ClientConfig, bool)> {
	let mut roots = rustls::RootCertStore::empty();
	if let Some(root) = material.root {
		let certs = CertificateDer::pem_slice_iter(&root).collect::<Result<Vec<_>, _>>()?;
		let (valid, invalid) = roots.add_parsable_certificates(certs);
		trace!(valid, invalid, "added root certificates")
	} else {
		// Skip unparsable native roots instead of panicking.
		let (valid, invalid) = roots.add_parsable_certificates(SYSTEM_ROOT.certs.clone());
		trace!(valid, invalid, "added system root certificates")
	}
	let roots = Arc::new(roots);
	let provider = transport::tls::provider_with_options_validated(
		&[],
		settings.key_exchange_groups.as_deref().unwrap_or_default(),
	)?;
	let ccb = ClientConfig::builder_with_provider(provider.clone())
		.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
		.expect("client config must be valid")
		.with_root_certificates(roots.clone());

	let mut cc = match (material.cert, material.key) {
		(Some(cert), Some(key)) => {
			let cert_chain = parse_cert(&cert)?;
			let private_key = parse_key(&key)?;
			ccb.with_client_auth_cert(cert_chain, private_key)?
		},
		(None, None) => ccb.with_no_client_auth(),
		// A cert without its key (or the reverse) would silently drop client auth.
		(Some(_), None) => {
			anyhow::bail!("backend TLS client certificate was set without a matching private key")
		},
		(None, Some(_)) => {
			anyhow::bail!("backend TLS private key was set without a matching client certificate")
		},
	};
	if settings.insecure_host {
		let inner =
			rustls::client::WebPkiServerVerifier::builder_with_provider(roots, provider).build()?;
		let verifier = Arc::new(tls::insecure::NoServerNameVerification::new(inner));
		cc.dangerous().set_certificate_verifier(verifier);
	} else if settings.insecure {
		cc.dangerous()
			.set_certificate_verifier(Arc::new(tls::insecure::NoVerifier));
	} else if let Some(alt_sans) = &settings.subject_alt_names {
		let sans = alt_sans
			.iter()
			.cloned()
			.map(tls::ExtendedServerName::try_from)
			.collect::<Result<Box<_>, _>>()?;
		cc.dangerous()
			.set_certificate_verifier(Arc::new(tls::insecure::AltHostnameVerifier::new(
				roots, sans,
			)));
	}
	cc.key_log = transport::tls::key_log();
	let allow_custom_alpn = settings.alpn.is_none();
	if let Some(a) = &settings.alpn {
		cc.alpn_protocols = a.iter().map(|b| b.as_bytes().to_vec()).collect();
	} else {
		cc.alpn_protocols = vec![b"h2".into(), b"http/1.1".into()];
	}
	Ok((cc, allow_custom_alpn))
}

fn reload_client_config(
	cert_path: &Option<PathBuf>,
	key_path: &Option<PathBuf>,
	root_path: &Option<PathBuf>,
	settings: &TlsSettings,
) -> anyhow::Result<(ClientConfig, bool)> {
	let material = CertMaterial {
		cert: cert_path.as_deref().map(fs_err::read).transpose()?,
		key: key_path.as_deref().map(fs_err::read).transpose()?,
		root: root_path.as_deref().map(fs_err::read).transpose()?,
	};
	build_client_config(material, settings)
}

/// Reloads the config when the files change; rotation doesn't produce a new xDS resource.
fn spawn_cert_file_watcher(
	config: &PerAlpnConfig,
	cert_path: Option<PathBuf>,
	key_path: Option<PathBuf>,
	root_path: Option<PathBuf>,
	settings: TlsSettings,
) -> PerAlpnConfig {
	// Plain unit tests have no Tokio runtime; skip the watcher there.
	let Ok(rt) = tokio::runtime::Handle::try_current() else {
		trace!("skipping backend TLS cert/key/root file watch: no Tokio runtime available");
		return config.clone();
	};
	let paths: Vec<PathBuf> = [&cert_path, &key_path, &root_path]
		.into_iter()
		.flatten()
		.cloned()
		.collect();
	let data = config.data_handle();
	let task = rt.spawn(async move {
		// Retry on watch errors (e.g. inotify limits) so hot-reload isn't lost for good.
		const RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
		loop {
			let mut watched = match crate::util::watch_files_with_options(
				paths.clone(),
				crate::util::WatchFilesOptions::default()
					// Rotation tools often replace files atomically; re-establish the watch on that.
					.reload_on_disappearance(true)
					.close_on_removal(true),
			) {
				Ok(w) => w,
				Err(e) => {
					warn!(
						"failed to watch backend TLS cert/key/root files, retrying in {RETRY_DELAY:?}: {e}"
					);
					tokio::time::sleep(RETRY_DELAY).await;
					continue;
				},
			};
			loop {
				let Some(invalidated) = watched.changed_invalidated().await else {
					warn!("backend TLS cert/key/root file watch closed unexpectedly, re-establishing");
					break;
				};
				match reload_client_config(&cert_path, &key_path, &root_path, &settings) {
					Ok((cc, allow_custom_alpn)) => {
						store_into(&data, Arc::new(cc), allow_custom_alpn);
						info!("reloaded backend TLS config after cert/key/root file change");
					},
					Err(e) => {
						warn!("failed to reload backend TLS cert/key/root files, keeping previous config: {e}");
					},
				}
				if invalidated {
					break;
				}
			}
		}
	});
	config.clone().with_watcher(task)
}

impl ResolvedBackendTLS {
	pub fn try_into(self) -> anyhow::Result<BackendTLS> {
		let metadata = BackendTLSInfo::from_resolved(&self);
		let hostname_override = self.hostname.map(|s| s.try_into()).transpose()?;

		let source: BackendTLSSource = if self.spiffe {
			if self.cert.is_some()
				|| self.key.is_some()
				|| self.root.is_some()
				|| self.insecure
				|| self.insecure_host
				|| self.key_exchange_groups.is_some()
				|| self.cert_path.is_some()
				|| self.key_path.is_some()
				|| self.root_path.is_some()
			{
				anyhow::bail!(
					"backend TLS 'spiffe' is mutually exclusive with 'cert'/'key'/'root'/'insecure'/'insecureHost'/'keyExchangeGroups'"
				);
			}
			BackendTLSSource::Spiffe(SpiffeBackendTLS {
				alpn: self.alpn,
				verify_sans: self.subject_alt_names.unwrap_or_default(),
			})
		} else {
			let should_watch =
				self.cert_path.is_some() || self.key_path.is_some() || self.root_path.is_some();
			// Files are re-read on every reload, so inline values would be lost.
			if should_watch && (self.cert.is_some() || self.key.is_some() || self.root.is_some()) {
				anyhow::bail!(
					"backend TLS cert/key/root must not mix inline values with file paths: once any of cert_path/key_path/root_path is set, cert/key/root must be unset"
				);
			}
			let settings = TlsSettings {
				insecure: self.insecure,
				insecure_host: self.insecure_host,
				subject_alt_names: self.subject_alt_names,
				key_exchange_groups: self.key_exchange_groups,
				alpn: self.alpn,
			};

			let (cc, allow_custom_alpn) = if should_watch {
				reload_client_config(&self.cert_path, &self.key_path, &self.root_path, &settings)?
			} else {
				build_client_config(
					CertMaterial {
						cert: self.cert,
						key: self.key,
						root: self.root,
					},
					&settings,
				)?
			};
			let config = PerAlpnConfig::new(Arc::new(cc), allow_custom_alpn);
			let config = if should_watch {
				spawn_cert_file_watcher(
					&config,
					self.cert_path,
					self.key_path,
					self.root_path,
					settings,
				)
			} else {
				config
			};
			BackendTLSSource::Static(config)
		};

		Ok(BackendTLS {
			hostname_override,
			source,
			metadata,
		})
	}
}

impl LocalBackendTLS {
	pub async fn try_into(
		self,
		resources: &crate::resource_manager::ResourceFetcher,
	) -> anyhow::Result<BackendTLS> {
		let cert = match self.cert {
			Some(path) => Some(
				resources
					.fetch(crate::resource_manager::ResourceRef::File(path))
					.await?
					.to_vec(),
			),
			None => None,
		};
		let key = match self.key {
			Some(path) => Some(
				resources
					.fetch(crate::resource_manager::ResourceRef::File(path))
					.await?
					.to_vec(),
			),
			None => None,
		};
		let root = match self.root {
			Some(path) => Some(
				resources
					.fetch(crate::resource_manager::ResourceRef::File(path))
					.await?
					.to_vec(),
			),
			None => None,
		};

		ResolvedBackendTLS {
			cert,
			key,
			root,
			hostname: self.hostname,
			insecure: self.insecure,
			insecure_host: self.insecure_host,
			alpn: self.alpn,
			subject_alt_names: self.subject_alt_names,
			key_exchange_groups: self.key_exchange_groups,
			cert_path: None,
			key_path: None,
			root_path: None,
			spiffe: self.spiffe.is_some(),
		}
		.try_into()
	}
}

#[cfg(test)]
mod hot_reload_tests {
	use std::time::Duration;

	use rcgen::{CertificateParams, KeyPair};

	use super::*;

	fn self_signed_cert_key() -> (Vec<u8>, Vec<u8>) {
		let key = KeyPair::generate().unwrap();
		let params = CertificateParams::new(vec!["localhost".to_string()]).unwrap();
		let cert = params.self_signed(&key).unwrap();
		(cert.pem().into_bytes(), key.serialize_pem().into_bytes())
	}

	#[test]
	fn inline_cert_with_file_based_root_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let root_path = dir.path().join("root.pem");
		let (cert, key) = self_signed_cert_key();
		std::fs::write(&root_path, &cert).unwrap();

		let err = ResolvedBackendTLS {
			cert: Some(cert),
			key: Some(key),
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: None,
			key_path: None,
			root_path: Some(root_path),
			spiffe: false,
		}
		.try_into()
		.expect_err("inline cert/key alongside a file-based root must be rejected");
		assert!(
			err
				.to_string()
				.contains("must not mix inline values with file paths")
		);
	}

	#[test]
	fn cert_without_key_fails_instead_of_dropping_client_auth() {
		let (cert, _key) = self_signed_cert_key();

		let err = ResolvedBackendTLS {
			cert: Some(cert),
			key: None,
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: None,
			key_path: None,
			root_path: None,
			spiffe: false,
		}
		.try_into()
		.expect_err("cert without a matching key must be rejected, not silently dropped");
		assert!(err.to_string().contains("private key"));
	}

	#[test]
	fn key_without_cert_fails_instead_of_dropping_client_auth() {
		let (_cert, key) = self_signed_cert_key();

		let err = ResolvedBackendTLS {
			cert: None,
			key: Some(key),
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: None,
			key_path: None,
			root_path: None,
			spiffe: false,
		}
		.try_into()
		.expect_err("key without a matching cert must be rejected, not silently dropped");
		assert!(err.to_string().contains("client certificate"));
	}

	#[tokio::test]
	async fn reloads_client_cert_on_file_change() {
		let dir = tempfile::tempdir().unwrap();
		let cert_path = dir.path().join("cert.pem");
		let key_path = dir.path().join("key.pem");

		let (cert1, key1) = self_signed_cert_key();
		std::fs::write(&cert_path, &cert1).unwrap();
		std::fs::write(&key_path, &key1).unwrap();

		let tls: BackendTLS = ResolvedBackendTLS {
			cert: None,
			key: None,
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: Some(cert_path.clone()),
			key_path: Some(key_path.clone()),
			root_path: None,
			spiffe: false,
		}
		.try_into()
		.unwrap();

		let before = tls.base_config().config;

		// Let the watcher register before the files are rewritten.
		tokio::time::sleep(Duration::from_millis(300)).await;

		let (cert2, key2) = self_signed_cert_key();
		std::fs::write(&cert_path, &cert2).unwrap();
		std::fs::write(&key_path, &key2).unwrap();

		// Debounced by 250ms, so poll.
		let mut after = tls.base_config().config;
		for _ in 0..50 {
			if !Arc::ptr_eq(&before, &after) {
				break;
			}
			tokio::time::sleep(Duration::from_millis(100)).await;
			after = tls.base_config().config;
		}
		assert!(
			!Arc::ptr_eq(&before, &after),
			"expected backend TLS config to hot-reload after cert/key file rotation"
		);
	}

	#[tokio::test]
	async fn keeps_reloading_after_a_delete_and_recreate_rotation() {
		let dir = tempfile::tempdir().unwrap();
		let cert_path = dir.path().join("cert.pem");
		let key_path = dir.path().join("key.pem");

		let (cert1, key1) = self_signed_cert_key();
		std::fs::write(&cert_path, &cert1).unwrap();
		std::fs::write(&key_path, &key1).unwrap();

		let tls: BackendTLS = ResolvedBackendTLS {
			cert: None,
			key: None,
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: Some(cert_path.clone()),
			key_path: Some(key_path.clone()),
			root_path: None,
			spiffe: false,
		}
		.try_into()
		.unwrap();

		let initial = tls.base_config().config;
		tokio::time::sleep(Duration::from_millis(300)).await;

		// Delete and recreate, so the watcher has to re-establish its watch.
		let (cert2, key2) = self_signed_cert_key();
		std::fs::remove_file(&cert_path).unwrap();
		std::fs::remove_file(&key_path).unwrap();
		std::fs::write(&cert_path, &cert2).unwrap();
		std::fs::write(&key_path, &key2).unwrap();

		let mut after_recreate = tls.base_config().config;
		for _ in 0..50 {
			if !Arc::ptr_eq(&initial, &after_recreate) {
				break;
			}
			tokio::time::sleep(Duration::from_millis(100)).await;
			after_recreate = tls.base_config().config;
		}
		assert!(
			!Arc::ptr_eq(&initial, &after_recreate),
			"expected reload after delete-and-recreate rotation"
		);

		// Let the re-established watch settle before the next rotation.
		tokio::time::sleep(Duration::from_millis(300)).await;

		let (cert3, key3) = self_signed_cert_key();
		std::fs::write(&cert_path, &cert3).unwrap();
		std::fs::write(&key_path, &key3).unwrap();

		let mut after_second_rotation = tls.base_config().config;
		for _ in 0..50 {
			if !Arc::ptr_eq(&after_recreate, &after_second_rotation) {
				break;
			}
			tokio::time::sleep(Duration::from_millis(100)).await;
			after_second_rotation = tls.base_config().config;
		}
		assert!(
			!Arc::ptr_eq(&after_recreate, &after_second_rotation),
			"watcher must still be running after a delete-and-recreate rotation, not stopped"
		);
	}

	#[tokio::test]
	async fn watcher_task_stops_once_config_is_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let cert_path = dir.path().join("cert.pem");
		let key_path = dir.path().join("key.pem");
		let (cert, key) = self_signed_cert_key();
		std::fs::write(&cert_path, &cert).unwrap();
		std::fs::write(&key_path, &key).unwrap();

		let tls: BackendTLS = ResolvedBackendTLS {
			cert: None,
			key: None,
			root: None,
			hostname: None,
			insecure: true,
			insecure_host: false,
			alpn: None,
			subject_alt_names: None,
			key_exchange_groups: None,
			cert_path: Some(cert_path),
			key_path: Some(key_path),
			root_path: None,
			spiffe: false,
		}
		.try_into()
		.unwrap();

		let weak = {
			let BackendTLSSource::Static(config) = &tls.source else {
				panic!("file-sourced backend TLS must be static");
			};
			config
				.watcher_weak()
				.expect("watcher should be attached while the config is alive")
		};
		assert!(
			weak.upgrade().is_some(),
			"watcher guard should still be alive"
		);

		drop(tls);

		assert!(
			weak.upgrade().is_none(),
			"dropping every clone of BackendTLS must drop (and abort) its watcher guard"
		);
	}
}

#[cfg(all(test, feature = "fips"))]
mod tests {
	use super::*;

	#[test]
	fn fips_config_backend_tls_rejects_non_approved_key_exchange_group() {
		let result = ResolvedBackendTLS {
			insecure: true,
			key_exchange_groups: Some(vec![tls::KeyExchangeGroup::X25519]),
			..Default::default()
		}
		.try_into();
		let err = match result {
			Ok(_) => panic!("non-approved backend key exchange group was accepted"),
			Err(err) => err,
		};
		assert!(
			err.to_string().contains("X25519"),
			"error should name the configured key exchange group, got: {err}"
		);
	}
}
