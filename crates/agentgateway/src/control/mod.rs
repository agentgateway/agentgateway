use std::io;
use std::io::Cursor;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use ::http::Uri;
use ::http::uri::Authority;
use agent_xds::ClientTrait;
use anyhow::Error;
use rustls::ClientConfig;
use secrecy::{ExposeSecret, SecretString};
use tonic::body::Body;
use tower::Service;

use crate::client::{ApplicationTransport, Transport};
use crate::http::HeaderValue;
use crate::http::backendtls::{BackendTLS, PerAlpnConfig, SYSTEM_TRUST};
use crate::types::agent::Target;
use crate::*;

pub mod caclient;

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub enum RootCert {
	File(PathBuf),
	Static(#[serde(skip)] Bytes),
	Default,
}

impl RootCert {
	pub async fn to_client_config(&self) -> anyhow::Result<BackendTLS> {
		let roots = match self {
			RootCert::File(f) => {
				let certfile = tokio::fs::read(f).await?;
				let mut reader = std::io::BufReader::new(Cursor::new(certfile));
				let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
				let mut roots = rustls::RootCertStore::empty();
				roots.add_parsable_certificates(certs);
				roots
			},
			RootCert::Static(b) => {
				let mut reader = std::io::BufReader::new(Cursor::new(b));
				let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
				let mut roots = rustls::RootCertStore::empty();
				roots.add_parsable_certificates(certs);
				roots
			},
			RootCert::Default => return Ok(SYSTEM_TRUST.clone()),
		};
		let mut ccb = ClientConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)?
			.with_root_certificates(roots)
			.with_no_client_auth();
		ccb.alpn_protocols = vec![b"h2".to_vec()];
		Ok(BackendTLS {
			hostname_override: None,
			config: PerAlpnConfig::new(Arc::new(ccb), false),
		})
	}
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub enum AuthSource {
	// JWT authentication source which contains the token file path and the cluster id.
	Token(PathBuf, String),
	// JWT authentication source which contains a static token file.
	// Note that this token is not refreshed, so its lifetime ought to be longer than ztunnel's
	StaticToken(#[serde(serialize_with = "ser_redact")] SecretString, String),
	None,
}

#[derive(serde::Serialize, Clone)]
struct AuthSourceLoaderInner {
	cluster_id: String,
	#[serde(serialize_with = "ser_redact")]
	current_token: Arc<RwLock<Arc<Vec<u8>>>>,
}

impl fmt::Debug for AuthSourceLoaderInner {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("AuthSourceLoader")
			.field("cluster_id", &self.cluster_id)
			.field("current_token", &"<redacted>")
			.finish()
	}
}

#[derive(serde::Serialize, Debug)]
pub struct AuthSourceLoader {
	inner: Option<AuthSourceLoaderInner>,
	#[serde(skip)]
	drop_notifier: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for AuthSourceLoader {
	fn drop(&mut self) {
		if let Some(tx) = self.drop_notifier.take() {
			let _ = tx.send(());
		}
	}
}

impl AuthSourceLoader {
	pub async fn new(auth: AuthSource) -> anyhow::Result<AuthSourceLoader> {
		let mut interval = tokio::time::interval(Duration::from_secs(60));
		interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		Self::new_with_interval(auth, interval).await
	}
	async fn new_with_interval(
		auth: AuthSource,
		mut interval: tokio::time::Interval,
	) -> anyhow::Result<AuthSourceLoader> {
		Ok(match auth {
			AuthSource::Token(path, cluster_id) => {
				let mut current_token = Self::load_token(&path).await?;
				let ret = AuthSourceLoaderInner {
					cluster_id,
					current_token: Arc::new(RwLock::new(Arc::new(Self::to_bearer(
						current_token.as_slice(),
					)))),
				};
				let (tx, mut rx) = tokio::sync::oneshot::channel();
				let token_pointer = ret.current_token.clone();
				tokio::spawn(async move {
					loop {
						tokio::select! {
							_ = &mut rx => {
								// Received shutdown signal
								return;
							}
							_ = interval.tick() => {}
						}
						let new_token = match Self::load_token(&path).await {
							Ok(t) => t,
							Err(e) => {
								tracing::error!("Failed to reload token from file {}: {}", path.display(), e);
								continue;
							},
						};

						if new_token != current_token {
							current_token = new_token;
							*token_pointer.write().unwrap().deref_mut() =
								Arc::new(Self::to_bearer(current_token.as_slice()));
						}
					}
				});
				AuthSourceLoader {
					inner: Some(ret),
					drop_notifier: Some(tx),
				}
			},
			AuthSource::StaticToken(token, cluster_id) => AuthSourceLoader {
				inner: Some(AuthSourceLoaderInner {
					cluster_id,
					current_token: Arc::new(RwLock::new(Arc::new(Self::to_bearer(
						token.expose_secret().as_bytes(),
					)))),
				}),
				drop_notifier: None,
			},
			AuthSource::None => AuthSourceLoader {
				inner: None,
				drop_notifier: None,
			},
		})
	}

	fn to_bearer(token: &[u8]) -> Vec<u8> {
		const BEARER_PREFIX: &[u8] = b"Bearer ";
		let mut bearer: Vec<u8> = Vec::with_capacity(BEARER_PREFIX.len() + token.len());
		bearer.extend_from_slice(BEARER_PREFIX);
		bearer.extend_from_slice(token);
		bearer
	}

	pub fn insert_headers(&self, request: &mut http::HeaderMap) -> anyhow::Result<()> {
		const AUTHORIZATION: &str = "authorization";
		const CLUSTER: &str = "clusterid";
		match &self.inner {
			Some(inner) => {
				let token = { inner.current_token.read().unwrap().clone() };
				let mut hv: HeaderValue = token.as_slice().try_into()?;
				hv.set_sensitive(true);
				request.insert(AUTHORIZATION, hv);
				request.insert(CLUSTER, inner.cluster_id.as_str().try_into()?);
				Ok(())
			},
			None => Ok(()),
		}
	}

	async fn load_token(path: &PathBuf) -> io::Result<Vec<u8>> {
		let t = tokio::fs::read(path).await?;

		if t.is_empty() {
			return Err(io::Error::other("token file exists, but was empty"));
		}
		Ok(t)
	}
}

pub async fn grpc_connector(
	client: client::Client,
	url: String,
	auth: AuthSource,
	root: RootCert,
) -> anyhow::Result<GrpcChannel> {
	let root = root.to_client_config().await?;
	let (target, transport) = get_target(&url, root)?;

	Ok(GrpcChannel {
		target,
		transport,
		client,
		auth: Arc::new(AuthSourceLoader::new(auth).await?),
	})
}

#[derive(Clone, Debug)]
pub struct GrpcChannel {
	target: Target,
	transport: Transport,
	client: client::Client,
	auth: Arc<AuthSourceLoader>,
}

impl tower::Service<::http::Request<tonic::body::Body>> for GrpcChannel {
	type Response = http::Response;
	type Error = anyhow::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Ok(()).into()
	}

	fn call(&mut self, req: ::http::Request<tonic::body::Body>) -> Self::Future {
		let client = self.client.clone();
		let auth = self.auth.clone();
		let target = self.target.clone();
		let transport = self.transport.clone();
		let mut req = req.map(http::Body::new);

		Box::pin(async move {
			auth.insert_headers(req.headers_mut())?;
			http::modify_req_uri(&mut req, |uri| {
				uri.authority = Some(Authority::try_from(target.to_string())?);
				uri.scheme = Some(transport.scheme());
				Ok(())
			})?;
			Ok(
				client
					.call(client::Call {
						req,
						target,
						transport,
					})
					.await?,
			)
		})
	}
}

impl agent_xds::ClientTrait for GrpcChannel {
	fn make_call(
		&mut self,
		req: ::http::Request<Body>,
	) -> Pin<Box<dyn Future<Output = Result<::http::Response<axum_core::body::Body>, Error>> + Send>>
	{
		self.call(req)
	}

	fn box_clone(&self) -> Box<dyn ClientTrait> {
		Box::new(self.clone())
	}
}

fn get_target(raw: &str, ca: BackendTLS) -> anyhow::Result<(Target, Transport)> {
	let uri = raw.parse::<Uri>()?;

	let target = if let Some(authority) = uri.authority() {
		Target::try_from(authority.to_string().as_str())?
	} else {
		anyhow::bail!("URI must have authority")
	};

	let transport = match uri.scheme_str() {
		Some("http") => ApplicationTransport::Plaintext.into(),
		Some("https") => ApplicationTransport::Tls(ca.base_config()).into(),
		_ => anyhow::bail!("Unsupported scheme: {}", uri.scheme_str().unwrap_or("none")),
	};

	Ok((target, transport))
}

#[cfg(test)]
mod tests {

	use std::fs::File;
	use std::io::Write;

	use secrecy::SecretString;
	use tempfile::tempdir;

	use super::*;

	#[tokio::test]
	async fn test_to_bearer() {
		let token = b"mytoken".to_vec();
		let bearer = AuthSourceLoader::to_bearer(token.as_slice());
		assert_eq!(bearer, b"Bearer mytoken".to_vec());
	}

	#[tokio::test]
	async fn test_static_token_loader_and_headers() {
		let token = SecretString::new("static-token-value".into());
		let cluster_id = "test-cluster".to_string();
		let loader = AuthSourceLoader::new(AuthSource::StaticToken(token.clone(), cluster_id.clone()))
			.await
			.unwrap();

		let mut headers = http::HeaderMap::new();
		loader.insert_headers(&mut headers).unwrap();

		let auth_header = headers.get("authorization").unwrap();
		assert_eq!(
			auth_header,
			&http::HeaderValue::from_static("Bearer static-token-value")
		);
		let cluster_header = headers.get("clusterid").unwrap();
		assert_eq!(
			cluster_header,
			&http::HeaderValue::from_static("test-cluster")
		);
	}

	#[tokio::test]
	async fn test_token_file_loader_and_headers() {
		let dir = tempdir().unwrap();
		let file_path = dir.path().join("token.txt");
		{
			let mut file = File::create(&file_path).unwrap();
			write!(file, "file-token-value").unwrap();
		}

		let cluster_id = "file-cluster".to_string();
		let loader = AuthSourceLoader::new(AuthSource::Token(file_path.clone(), cluster_id.clone()))
			.await
			.unwrap();

		let mut headers = http::HeaderMap::new();
		loader.insert_headers(&mut headers).unwrap();

		let mut auth_header = headers.get("authorization").unwrap().clone();
		auth_header.set_sensitive(false);
		assert_eq!(
			auth_header,
			&http::HeaderValue::from_static("Bearer file-token-value")
		);
		let cluster_header = headers.get("clusterid").unwrap();
		assert_eq!(
			cluster_header,
			&http::HeaderValue::from_static("file-cluster")
		);
	}

	#[tokio::test]
	async fn test_token_file_loader_rotation() {
		let dir = tempdir().unwrap();
		let file_path = dir.path().join("token.txt");
		{
			let mut file = File::create(&file_path).unwrap();
			write!(file, "file-token-value").unwrap();
		}

		let interval: tokio::time::Interval = tokio::time::interval(Duration::from_millis(10));
		let loader = AuthSourceLoader::new_with_interval(
			AuthSource::Token(file_path.clone(), "file-cluster".to_string()),
			interval,
		)
		.await
		.unwrap();
		{
			let mut file = File::create(&file_path).unwrap();
			write!(file, "file-token-value-2").unwrap();
		}

		tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

		let mut headers = http::HeaderMap::new();
		loader.insert_headers(&mut headers).unwrap();

		let auth_header = headers.get("authorization").unwrap();
		assert_eq!(
			auth_header,
			&http::HeaderValue::from_static("Bearer file-token-value-2")
		);
	}

	#[tokio::test]
	async fn test_none_auth_source_loader() {
		let loader = AuthSourceLoader::new(AuthSource::None).await.unwrap();
		let mut headers = http::HeaderMap::new();
		loader.insert_headers(&mut headers).unwrap();
		assert!(headers.get("authorization").is_none());
		assert!(headers.get("clusterid").is_none());
	}
}
