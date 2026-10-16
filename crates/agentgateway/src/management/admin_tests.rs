use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::BodyExt;
use tokio::runtime::Handle;

use super::*;

async fn spawn_admin(cfg: &str) -> (SocketAddr, agent_core::drain::DrainTrigger) {
	let config = Arc::new(crate::config::parse_config(cfg.to_string(), None).unwrap());
	let stores = crate::store::Stores::new(config.ipv6_enabled, config.threading_mode);
	let client = crate::client::Client::new(&config.dns, None, Default::default(), None);
	let resource_manager =
		crate::resource_manager::ResourceManager::new(client).expect("resource manager");
	let shutdown = signal::Shutdown::new();
	let (drain_tx, drain_rx) = agent_core::drain::new();
	let svc = Service::new(
		config,
		crate::llm::catalog::ModelCatalog::empty(),
		None,
		stores,
		resource_manager,
		shutdown.trigger(),
		drain_rx,
		Handle::current(),
		&crate::ui::EMPTY_ASSETS_DIR,
	)
	.await
	.expect("admin server should bind");
	let addr = svc.address().expect("admin server should have an address");
	svc.spawn();
	(addr, drain_tx)
}

#[tokio::test]
async fn test_admin_config_dump_redacts_secrets() {
	let cfg = r#"
config:
  adminAddr: localhost:0
  tracing:
    otlpEndpoint: http://localhost:4317
    headers:
      authorization: super-secret-otlp-token
      x-custom-header: visible-value
"#;
	let (addr, _drain_tx) = spawn_admin(cfg).await;

	let resp = reqwest::get(format!("http://{addr}/config_dump"))
		.await
		.expect("request should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);

	let body = resp.text().await.unwrap();
	assert!(
		!body.contains("super-secret-otlp-token"),
		"config dump must not leak authorization header value: {body}"
	);
	assert!(
		body.contains("visible-value"),
		"config dump should preserve non-sensitive header values: {body}"
	);
}

#[tokio::test]
async fn trace_sse_stream_does_not_repoll_after_eof() {
	let stream = trace_sse_stream(crate::proxy::dtrace::TraceReceiver::closed_for_test());
	let mut body = crate::http::Body::from_stream(stream);

	assert!(
		body.frame().await.is_some(),
		"ready frame should be present"
	);
	assert!(body.frame().await.is_none(), "stream should report EOF");
	assert!(body.frame().await.is_none(), "stream must remain at EOF");
}

struct TestPki {
	dir: tempfile::TempDir,
	ca_pem: String,
	client_identity_pem: String,
	untrusted_client_identity_pem: String,
}

fn ca_params(name: &str) -> rcgen::CertificateParams {
	let mut params = rcgen::CertificateParams::new(vec![]).unwrap();
	params
		.distinguished_name
		.push(rcgen::DnType::CommonName, name);
	params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
	params.key_usages = vec![
		rcgen::KeyUsagePurpose::KeyCertSign,
		rcgen::KeyUsagePurpose::DigitalSignature,
	];
	params
}

fn leaf(
	name: &str,
	usage: rcgen::ExtendedKeyUsagePurpose,
	issuer: &rcgen::Issuer<'_, rcgen::KeyPair>,
) -> (String, String) {
	let mut params = rcgen::CertificateParams::new(vec![name.to_string()]).unwrap();
	params.extended_key_usages = vec![usage];
	let key = rcgen::KeyPair::generate().unwrap();
	let cert = params.signed_by(&key, issuer).unwrap();
	(cert.pem(), key.serialize_pem())
}

impl TestPki {
	fn new() -> Self {
		let ca_key = rcgen::KeyPair::generate().unwrap();
		let ca_cert = ca_params("admin test ca").self_signed(&ca_key).unwrap();
		let ca = rcgen::Issuer::new(ca_params("admin test ca"), ca_key);
		let other_key = rcgen::KeyPair::generate().unwrap();
		let other = rcgen::Issuer::new(ca_params("untrusted ca"), other_key);

		let (server_cert, server_key) =
			leaf("localhost", rcgen::ExtendedKeyUsagePurpose::ServerAuth, &ca);
		let (client_cert, client_key) = leaf("client", rcgen::ExtendedKeyUsagePurpose::ClientAuth, &ca);
		let (bad_cert, bad_key) = leaf("client", rcgen::ExtendedKeyUsagePurpose::ClientAuth, &other);

		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("cert.pem"), server_cert).unwrap();
		std::fs::write(dir.path().join("key.pem"), server_key).unwrap();
		let ca_pem = ca_cert.pem();
		std::fs::write(dir.path().join("ca.pem"), &ca_pem).unwrap();
		Self {
			dir,
			ca_pem,
			client_identity_pem: format!("{client_cert}{client_key}"),
			untrusted_client_identity_pem: format!("{bad_cert}{bad_key}"),
		}
	}

	fn path(&self, name: &str) -> String {
		self.dir.path().join(name).display().to_string()
	}

	fn config(&self, client_ca: bool) -> String {
		let client_ca = if client_ca {
			format!("\n    clientCa: {}", self.path("ca.pem"))
		} else {
			String::new()
		};
		format!(
			"config:\n  adminAddr: 127.0.0.1:0\n  adminTls:\n    cert: {}\n    key: {}{client_ca}\n",
			self.path("cert.pem"),
			self.path("key.pem"),
		)
	}

	fn client(&self, identity: Option<&str>) -> reqwest::Client {
		let mut builder = reqwest::Client::builder()
			.add_root_certificate(reqwest::Certificate::from_pem(self.ca_pem.as_bytes()).unwrap())
			.resolve("localhost", ([127, 0, 0, 1], 0).into());
		if let Some(identity) = identity {
			builder = builder.identity(reqwest::Identity::from_pem(identity.as_bytes()).unwrap());
		}
		builder.build().unwrap()
	}
}

#[tokio::test]
async fn admin_serves_https_when_tls_configured() {
	let pki = TestPki::new();
	let (addr, _drain_tx) = spawn_admin(&pki.config(false)).await;

	let resp = pki
		.client(None)
		.get(format!("https://localhost:{}/config_dump", addr.port()))
		.send()
		.await
		.expect("https request should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn admin_plain_http_fails_when_tls_configured() {
	let pki = TestPki::new();
	let (addr, _drain_tx) = spawn_admin(&pki.config(false)).await;

	let result = reqwest::get(format!("http://{addr}/config_dump")).await;
	assert!(result.is_err(), "plain http must not be served over TLS");

	let resp = pki
		.client(None)
		.get(format!("https://localhost:{}/config_dump", addr.port()))
		.send()
		.await
		.expect("listener must keep accepting after a failed handshake");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn admin_client_ca_requires_valid_client_certificate() {
	let pki = TestPki::new();
	let (addr, _drain_tx) = spawn_admin(&pki.config(true)).await;
	let url = format!("https://localhost:{}/config_dump", addr.port());

	assert!(
		pki.client(None).get(&url).send().await.is_err(),
		"request without a client certificate must be rejected"
	);
	assert!(
		pki
			.client(Some(&pki.untrusted_client_identity_pem))
			.get(&url)
			.send()
			.await
			.is_err(),
		"request with an untrusted client certificate must be rejected"
	);
	let resp = pki
		.client(Some(&pki.client_identity_pem))
		.get(&url)
		.send()
		.await
		.expect("request with a valid client certificate should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn admin_plain_http_unchanged_without_tls() {
	let (addr, _drain_tx) = spawn_admin("config:\n  adminAddr: 127.0.0.1:0\n").await;
	let resp = reqwest::get(format!("http://{addr}/config_dump"))
		.await
		.expect("plain http should succeed");
	assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[test]
fn admin_tls_config_validation() {
	let pki = TestPki::new();
	let parse = |tls: &str| {
		crate::config::parse_config(
			format!("config:\n  adminAddr: 127.0.0.1:0\n  adminTls:\n{tls}"),
			None,
		)
	};
	let cert = pki.path("cert.pem");
	let key = pki.path("key.pem");
	let ca = pki.path("ca.pem");

	let err = parse(&format!("    cert: {cert}\n")).unwrap_err();
	assert!(err.to_string().contains("must be set together"), "{err}");
	let err = parse(&format!("    key: {key}\n")).unwrap_err();
	assert!(err.to_string().contains("must be set together"), "{err}");
	let err = parse(&format!("    clientCa: {ca}\n")).unwrap_err();
	assert!(err.to_string().contains("requires"), "{err}");
	let cfg = parse(&format!(
		"    cert: {cert}\n    key: {key}\n    clientCa: {ca}\n"
	))
	.unwrap();
	assert_eq!(
		cfg.admin_tls,
		Some(crate::AdminTlsConfig {
			cert: cert.into(),
			key: key.into(),
			client_ca: Some(ca.into()),
		})
	);

	let err = crate::config::parse_config(
		format!(
			"config:\n  adminAddr: unix:/tmp/admin.sock\n  adminTls:\n    cert: {}\n    key: {}\n",
			pki.path("cert.pem"),
			pki.path("key.pem")
		),
		None,
	)
	.unwrap_err();
	assert!(err.to_string().contains("Unix socket"), "{err}");
}
