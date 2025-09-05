use std::io;
use std::io::Cursor;
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

use crate::client::Transport;
use crate::http::HeaderValue;
use crate::http::backendtls::{BackendTLS, SYSTEM_TRUST};
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
			config: Arc::new(ccb),
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

impl AuthSource {
	pub async fn insert_headers(&self, request: &mut http::HeaderMap) -> anyhow::Result<()> {
		const AUTHORIZATION: &str = "authorization";
		const CLUSTER: &str = "clusterid";
		match self {
			AuthSource::Token(path, cluster_id) => {
				let token = load_token(path).await.map(|mut t| {
					let mut bearer: Vec<u8> = b"Bearer ".to_vec();
					bearer.append(&mut t);
					bearer
				})?;
				let mut hv: HeaderValue = token.try_into()?;
				hv.set_sensitive(true);
				request.insert(AUTHORIZATION, hv);
				request.insert(CLUSTER, cluster_id.try_into()?);
			},
			AuthSource::StaticToken(token, cluster_id) => {
				let token = {
					let mut bearer: Vec<u8> = b"Bearer ".to_vec();
					bearer.extend_from_slice(token.expose_secret().as_bytes());
					bearer
				};
				let mut hv: HeaderValue = token.try_into()?;
				hv.set_sensitive(true);
				request.insert(AUTHORIZATION, hv);
				request.insert(CLUSTER, cluster_id.try_into()?);
			},
			AuthSource::None => {},
		}
		Ok(())
	}
}

async fn load_token(path: &PathBuf) -> io::Result<Vec<u8>> {
	let t = tokio::fs::read(path).await?;

	if t.is_empty() {
		return Err(io::Error::other("token file exists, but was empty"));
	}
	Ok(t)
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
		auth: Arc::new(auth),
	})
}

#[derive(Clone, Debug)]
pub struct GrpcChannel {
	target: Target,
	transport: Transport,
	client: client::Client,
	auth: Arc<AuthSource>,
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
			auth.insert_headers(req.headers_mut()).await?;
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
		Some("http") => Transport::Plaintext,
		Some("https") => Transport::Tls(ca),
		_ => anyhow::bail!("Unsupported scheme: {}", uri.scheme_str().unwrap_or("none")),
	};

	Ok((target, transport))
}
