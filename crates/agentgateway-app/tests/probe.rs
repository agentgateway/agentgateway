use std::net::SocketAddr;

use agent_core::{drain, readiness};
use agentgateway::Address;
use agentgateway::management::readiness_server::Server;
use tokio::process::Command;

fn probe(address: SocketAddr) -> Command {
	let mut command = Command::new(env!("CARGO_BIN_EXE_agentgateway"));
	command
		.args(["probe", "--address", &address.to_string()])
		.env("KUBERNETES_SERVICE_HOST", "test")
		.env_remove("LOCAL_XDS_PATH")
		.kill_on_drop(true);
	command
}

#[tokio::test]
async fn probe_checks_loopback_readiness() {
	let (_drain, watcher) = drain::new();
	let ready = readiness::Ready::new();
	let startup = ready.register_task("startup");
	let server = Server::new(Address::Localhost(false, 0), watcher, ready.clone())
		.await
		.unwrap();
	let address = server.address().unwrap();
	server.spawn();
	drop(startup);

	let output = probe(address).output().await.unwrap();
	assert_eq!(
		output.status.code(),
		Some(0),
		"probe failed against a ready loopback listener: {}",
		String::from_utf8_lossy(&output.stderr),
	);

	let _pending = ready.register_task("reload");
	let output = probe(address).output().await.unwrap();
	assert_eq!(output.status.code(), Some(1));
	assert!(String::from_utf8_lossy(&output.stderr).contains("500 Internal Server Error"));
}

#[tokio::test]
async fn probe_times_out_waiting_for_response() {
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let output = tokio::time::timeout(
		std::time::Duration::from_secs(10),
		probe(listener.local_addr().unwrap()).output(),
	)
	.await
	.expect("probe did not time out")
	.unwrap();
	assert_eq!(output.status.code(), Some(1));
	assert!(String::from_utf8_lossy(&output.stderr).contains("timed out"));
}
