use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use agent_core::strng;
use agent_core::strng::Strng;
use arc_swap::ArcSwap;
use once_cell::sync::Lazy;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use serde::Serializer;
use tracing::{info, trace, warn};

use crate::serdes::{schema_de, schema_ser};
use crate::transport::tls;
use crate::types::agent::{parse_cert, parse_key};
use crate::types::discovery::Identity;
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
	/// The configured TLS could not be built. Connections fail rather than fall back to plaintext.
	Invalid(Strng),
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
			expected_identities: None,
			peer_identity_mode: tls::PeerIdentityMode::Istio,
		}
	}
}

#[derive(Debug, Clone)]
pub struct VersionedBackendTLS {
	pub hostname_override: Option<ServerName<'static>>,
	pub config: Arc<ClientConfig>,
	/// If set, `config` is shared across identity sets and the peer is verified against these
	/// identities with a verifier installed per connection. See `connect_config`.
	pub expected_identities: Option<ExpectedIdentities>,
	/// How to interpret the upstream server's SPIFFE identity when extracting peer TLS info.
	/// `Spiffe` for SPIFFE-sourced backends so their non-Istio SVIDs are not parsed as Istio identities.
	pub peer_identity_mode: tls::PeerIdentityMode,
}

#[derive(Debug, Clone)]
pub struct ExpectedIdentities {
	pub roots: Arc<RootCertStore>,
	pub identities: Vec<Identity>,
}

impl VersionedBackendTLS {
	/// The config to establish a new connection with.
	pub fn connect_config(&self) -> Arc<ClientConfig> {
		let Some(expected) = &self.expected_identities else {
			return self.config.clone();
		};
		let mut cc = ClientConfig::clone(&self.config);
		cc.dangerous()
			.set_certificate_verifier(Arc::new(tls::identity::IdentityVerifier {
				roots: expected.roots.clone(),
				identity: expected.identities.clone(),
			}));
		Arc::new(cc)
	}
}

// Used in the connection pool key, so it must cover everything that affects how the connection was verified.
// `roots` is not included as it is tied to `config`.
impl std::hash::Hash for VersionedBackendTLS {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		// Hash the pointer address
		Arc::as_ptr(&self.config).hash(state);
		self.hostname_override.hash(state);
		self
			.expected_identities
			.as_ref()
			.map(|e| &e.identities)
			.hash(state);
	}
}

impl PartialEq for VersionedBackendTLS {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.config, &other.config)
			&& self.hostname_override == other.hostname_override
			&& self.expected_identities.as_ref().map(|e| &e.identities)
				== other.expected_identities.as_ref().map(|e| &e.identities)
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
	pub cert_path: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub key_path: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub root_path: Option<String>,
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
			cert_path: tls.cert_path.as_deref().map(path_to_string),
			key_path: tls.key_path.as_deref().map(path_to_string),
			root_path: tls.root_path.as_deref().map(path_to_string),
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

fn path_to_string(path: &std::path::Path) -> String {
	path.to_string_lossy().into_owned()
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
		_ => ccb.with_no_client_auth(),
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

/// Directories that file-sourced backend TLS material must live under, from the gateway config.
/// Empty means no file may be used.
static ALLOWED_FILE_DIRS: Lazy<ArcSwap<Vec<PathBuf>>> = Lazy::new(|| ArcSwap::from_pointee(vec![]));

pub fn set_allowed_file_dirs(dirs: Vec<PathBuf>) {
	ALLOWED_FILE_DIRS.store(Arc::new(dirs));
}

pub fn allowed_file_dirs() -> Arc<Vec<PathBuf>> {
	ALLOWED_FILE_DIRS.load_full()
}

/// Reads `path` if it is under one of `allowed_dirs`, after resolving symlinks.
fn read_allowed_file(path: &Path, allowed_dirs: &[PathBuf]) -> anyhow::Result<Vec<u8>> {
	let not_allowed = || {
		anyhow::anyhow!(
			"backend TLS file {} is not under a directory allowed by config.allowedBackendTlsFileDirs",
			path.display()
		)
	};
	// Checked before touching the filesystem, so paths outside the allowed directories can't be probed.
	if !path.is_absolute()
		|| path.components().any(|c| c == Component::ParentDir)
		|| !allowed_dirs.iter().any(|dir| path.starts_with(dir))
	{
		return Err(not_allowed());
	}
	// A symlink inside an allowed directory must not lead outside of it.
	let resolved = fs_err::canonicalize(path)?;
	if !allowed_dirs
		.iter()
		.filter_map(|dir| fs_err::canonicalize(dir).ok())
		.any(|dir| resolved.starts_with(dir))
	{
		return Err(not_allowed());
	}
	Ok(fs_err::read(resolved)?)
}

/// Files a config reads its cert, key and root from, and the directories they must be under.
#[derive(Clone)]
struct FileSources {
	cert: Option<PathBuf>,
	key: Option<PathBuf>,
	root: Option<PathBuf>,
	allowed_dirs: Vec<PathBuf>,
}

impl FileSources {
	fn is_empty(&self) -> bool {
		self.cert.is_none() && self.key.is_none() && self.root.is_none()
	}

	fn paths(&self) -> Vec<PathBuf> {
		[&self.cert, &self.key, &self.root]
			.into_iter()
			.flatten()
			.cloned()
			.collect()
	}

	fn read(&self) -> anyhow::Result<CertMaterial> {
		anyhow::ensure!(
			self.cert.is_some() == self.key.is_some(),
			"backend TLS cert_path and key_path must be set together"
		);
		let read = |path: &Option<PathBuf>| {
			path
				.as_deref()
				.map(|path| read_allowed_file(path, &self.allowed_dirs))
				.transpose()
		};
		Ok(CertMaterial {
			cert: read(&self.cert)?,
			key: read(&self.key)?,
			root: read(&self.root)?,
		})
	}
}

fn reload_client_config(
	files: &FileSources,
	settings: &TlsSettings,
) -> anyhow::Result<(ClientConfig, bool)> {
	build_client_config(files.read()?, settings)
}

/// Reloads the config when the files change; rotation doesn't produce a new xDS resource.
fn spawn_cert_file_watcher(
	config: &PerAlpnConfig,
	files: FileSources,
	settings: TlsSettings,
) -> PerAlpnConfig {
	// Plain unit tests have no Tokio runtime; skip the watcher there.
	let Ok(rt) = tokio::runtime::Handle::try_current() else {
		trace!("skipping backend TLS cert/key/root file watch: no Tokio runtime available");
		return config.clone();
	};
	let paths = files.paths();
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
				match reload_client_config(&files, &settings) {
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
		self.try_into_with(&allowed_file_dirs())
	}

	/// Like `try_into`, with `allowed_file_dirs` limiting where `cert_path`/`key_path`/`root_path` may point.
	fn try_into_with(self, allowed_file_dirs: &[PathBuf]) -> anyhow::Result<BackendTLS> {
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
			let files = FileSources {
				cert: self.cert_path,
				key: self.key_path,
				root: self.root_path,
				allowed_dirs: allowed_file_dirs.to_vec(),
			};
			let should_watch = !files.is_empty();
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
				reload_client_config(&files, &settings)?
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
				spawn_cert_file_watcher(&config, files, settings)
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

	fn write_cert_key(dir: &Path) -> (PathBuf, PathBuf) {
		let (cert, key) = self_signed_cert_key();
		let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
		std::fs::write(&cert_path, cert).unwrap();
		std::fs::write(&key_path, key).unwrap();
		(cert_path, key_path)
	}

	fn from_files(allowed: &[&Path], cert: &Path, key: &Path) -> anyhow::Result<BackendTLS> {
		ResolvedBackendTLS {
			insecure: true,
			cert_path: Some(cert.to_path_buf()),
			key_path: Some(key.to_path_buf()),
			..Default::default()
		}
		.try_into_with(&allowed.iter().map(|d| d.to_path_buf()).collect::<Vec<_>>())
	}

	/// Polls until the config differs from `previous` (reloads are debounced by 250ms).
	async fn next_config(tls: &BackendTLS, previous: &Arc<ClientConfig>) -> Arc<ClientConfig> {
		let mut current = tls.base_config().config;
		for _ in 0..50 {
			if !Arc::ptr_eq(previous, &current) {
				break;
			}
			tokio::time::sleep(Duration::from_millis(100)).await;
			current = tls.base_config().config;
		}
		current
	}

	#[test]
	fn inline_cert_with_file_based_root_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let (cert_path, _) = write_cert_key(dir.path());
		let (cert, key) = self_signed_cert_key();

		let err = ResolvedBackendTLS {
			insecure: true,
			cert: Some(cert),
			key: Some(key),
			root_path: Some(cert_path),
			..Default::default()
		}
		.try_into_with(&[dir.path().to_path_buf()])
		.expect_err("inline cert/key alongside a file-based root must be rejected");
		assert!(
			err
				.to_string()
				.contains("must not mix inline values with file paths")
		);
	}

	#[test]
	fn cert_path_and_key_path_must_be_set_together() {
		let dir = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());

		for (cert, key) in [(Some(cert_path), None), (None, Some(key_path))] {
			let err = ResolvedBackendTLS {
				insecure: true,
				cert_path: cert,
				key_path: key,
				..Default::default()
			}
			.try_into_with(&[dir.path().to_path_buf()])
			.expect_err("a cert without a key (or the reverse) must be rejected");
			assert!(err.to_string().contains("must be set together"));
		}
	}

	#[test]
	fn files_must_be_under_an_allowed_dir() {
		let dir = tempfile::tempdir().unwrap();
		let other = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());

		from_files(&[dir.path()], &cert_path, &key_path).expect("file under an allowed dir");
		for allowed in [&[][..], &[other.path()][..]] {
			let err = from_files(allowed, &cert_path, &key_path).expect_err("file outside allowed dirs");
			assert!(err.to_string().contains("allowedBackendTlsFileDirs"));
		}

		let traversal = dir.path().join("../").join(dir.path().file_name().unwrap());
		let err = from_files(&[dir.path()], &traversal.join("cert.pem"), &key_path)
			.expect_err("`..` must be rejected");
		assert!(err.to_string().contains("allowedBackendTlsFileDirs"));
	}

	#[cfg(unix)]
	#[test]
	fn symlink_out_of_an_allowed_dir_is_rejected() {
		let dir = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		let (_, key_path) = write_cert_key(dir.path());
		let (outside_cert, _) = write_cert_key(outside.path());
		let link = dir.path().join("link.pem");
		std::os::unix::fs::symlink(&outside_cert, &link).unwrap();

		let err = from_files(&[dir.path()], &link, &key_path).expect_err("symlink escapes");
		assert!(err.to_string().contains("allowedBackendTlsFileDirs"));
	}

	#[tokio::test]
	async fn reloads_client_cert_on_file_change() {
		let dir = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());
		let tls = from_files(&[dir.path()], &cert_path, &key_path).unwrap();
		let before = tls.base_config().config;

		// Let the watcher register before the files are rewritten.
		tokio::time::sleep(Duration::from_millis(300)).await;
		write_cert_key(dir.path());

		let after = next_config(&tls, &before).await;
		assert!(
			!Arc::ptr_eq(&before, &after),
			"expected backend TLS config to hot-reload after cert/key file rotation"
		);
	}

	#[tokio::test]
	async fn keeps_reloading_after_a_delete_and_recreate_rotation() {
		let dir = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());
		let tls = from_files(&[dir.path()], &cert_path, &key_path).unwrap();
		let initial = tls.base_config().config;
		tokio::time::sleep(Duration::from_millis(300)).await;

		// Delete and recreate, so the watcher has to re-establish its watch.
		std::fs::remove_file(&cert_path).unwrap();
		std::fs::remove_file(&key_path).unwrap();
		write_cert_key(dir.path());
		let after_recreate = next_config(&tls, &initial).await;
		assert!(
			!Arc::ptr_eq(&initial, &after_recreate),
			"expected reload after delete-and-recreate rotation"
		);

		// Let the re-established watch settle before the next rotation.
		tokio::time::sleep(Duration::from_millis(300)).await;
		write_cert_key(dir.path());
		let after_second_rotation = next_config(&tls, &after_recreate).await;
		assert!(
			!Arc::ptr_eq(&after_recreate, &after_second_rotation),
			"watcher must still be running after a delete-and-recreate rotation, not stopped"
		);
	}

	#[cfg(unix)]
	#[tokio::test]
	async fn reload_keeps_previous_config_if_a_file_leaves_the_allowed_dir() {
		let dir = tempfile::tempdir().unwrap();
		let outside = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());
		let (outside_cert, _) = write_cert_key(outside.path());
		let tls = from_files(&[dir.path()], &cert_path, &key_path).unwrap();
		let before = tls.base_config().config;
		tokio::time::sleep(Duration::from_millis(300)).await;

		std::fs::remove_file(&cert_path).unwrap();
		std::os::unix::fs::symlink(&outside_cert, &cert_path).unwrap();

		let after = next_config(&tls, &before).await;
		assert!(
			Arc::ptr_eq(&before, &after),
			"a disallowed file must not be loaded"
		);
	}

	#[tokio::test]
	async fn watcher_task_stops_once_config_is_dropped() {
		let dir = tempfile::tempdir().unwrap();
		let (cert_path, key_path) = write_cert_key(dir.path());
		let tls = from_files(&[dir.path()], &cert_path, &key_path).unwrap();

		let BackendTLSSource::Static(config) = &tls.source else {
			panic!("expected a static backend TLS source");
		};
		let weak = config
			.watcher_weak()
			.expect("watcher should be attached while the config is alive");
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

	#[test]
	fn info_reports_file_paths_instead_of_pem() {
		let info = BackendTLSInfo::from_resolved(&ResolvedBackendTLS {
			cert_path: Some("/certs/tls.crt".into()),
			key_path: Some("/certs/tls.key".into()),
			root_path: Some("/certs/ca.crt".into()),
			..Default::default()
		});
		assert_eq!(info.cert_path.as_deref(), Some("/certs/tls.crt"));
		assert_eq!(info.key_path.as_deref(), Some("/certs/tls.key"));
		assert_eq!(info.root_path.as_deref(), Some("/certs/ca.crt"));
		assert!(info.cert.is_none() && info.root.is_none());
		assert!(!info.system_roots);
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
