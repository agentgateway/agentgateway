use super::server::MockMcpServer;

pub(crate) const TEST_SESSION_KEY: &str =
	"000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

pub(crate) fn multiplex_config(
	mcp1: &MockMcpServer,
	mcp2: &MockMcpServer,
	broken_mcp_addr: std::net::SocketAddr,
) -> String {
	format!(
		r#"
config:
  session:
    key: {TEST_SESSION_KEY}
binds:
- port: $PORT
  listeners:
  - name: comprehensive-gateway
    routes:
    - matches:
      - path:
          pathPrefix: /mcp
      backends:
      - mcp:
          allowDegraded: true
          targets:
          - name: s1
            mcp:
              host: http://{}/mcp
          - name: s2
            mcp:
              host: http://{}/mcp
          - name: s3
            mcp:
              host: http://{}/mcp
      policies:
        mcpAuthorization:
          rules:
          - 'true'
          - deny: 'mcp.tool.target == "s2" && mcp.tool.name == "echo"'
"#,
		mcp1.addr, mcp2.addr, broken_mcp_addr
	)
}

pub(crate) fn multiplex_transport_matrix_config(
	streamable: &MockMcpServer,
	sse: &MockMcpServer,
	broken_mcp_addr: std::net::SocketAddr,
) -> String {
	format!(
		r#"
config:
  session:
    key: {TEST_SESSION_KEY}
binds:
- port: $PORT
  listeners:
  - name: comprehensive-gateway
    routes:
    - matches:
      - path:
          pathPrefix: /mcp
      backends:
      - mcp:
          allowDegraded: true
          targets:
          - name: stream
            mcp:
              host: http://{}/mcp
          - name: sse
            sse:
              host: http://{}/sse
          - name: s3
            mcp:
              host: http://{}/mcp
      policies:
        mcpAuthorization:
          rules:
          - 'true'
"#,
		streamable.addr, sse.addr, broken_mcp_addr
	)
}

pub(crate) fn simple_multiplex_config(
	listener_name: &str,
	targets: &[(String, std::net::SocketAddr)],
) -> String {
	let mut config = format!(
		r#"config:
  session:
    key: {TEST_SESSION_KEY}
binds:
- port: $PORT
  listeners:
  - name: {listener_name}
    routes:
    - matches:
      - path:
          pathPrefix: /mcp
      backends:
      - mcp:
          allowDegraded: true
          targets:
"#
	);

	for (name, addr) in targets {
		config.push_str(&format!(
			"          - name: {name}\n            mcp:\n              host: http://{addr}/mcp\n"
		));
	}

	config
}
