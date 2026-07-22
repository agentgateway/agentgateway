fn main() -> Result<(), anyhow::Error> {
	let cwd = std::env::current_dir()?;
	let proto_names = [
		"proto/shared_envoy.proto",
		"proto/xds.proto",
		"proto/citadel.proto",
		"proto/ext_authz.proto",
		"proto/ext_mcp.proto",
		"proto/ext_proc.proto",
		"proto/rls.proto",
		"proto/workload.proto",
		"proto/resource.proto",
		// SPIFFE Workload API. The generated bindings are gated behind the `spiffe-test-server`
		// feature in lib.rs (see the `spiffe_workload_api` module) so they are only compiled into
		// the crate for tests; generating the descriptor here unconditionally is harmless and keeps
		// the build script free of feature-name coupling.
		"proto/spiffe_workload.proto",
	];
	let proto_files = proto_names
		.iter()
		.map(|name| cwd.join(name))
		.collect::<Vec<_>>();
	let include_dirs = [cwd.join("proto")];

	let config = {
		let mut c = prost_build::Config::new();
		c.disable_comments(Some("."));
		c.bytes([
			".istio.workload.Workload",
			".istio.workload.Service",
			".istio.workload.GatewayAddress",
			".istio.workload.Address",
			".envoy.service.auth.v3.AttributeContext.HttpRequest.raw_body",
			".envoy.service.ext_proc.v3.HttpBody.body",
			".envoy.service.ext_proc.v3.BodyMutation.body",
			".envoy.service.ext_proc.v3.StreamedBodyResponse.body",
			".agentgateway.dev.ext_mcp.McpRequest.mcp_request",
			".agentgateway.dev.ext_mcp.McpResponse.mcp_response",
			".agentgateway.dev.ext_mcp.McpRequestResult.mutated",
			".agentgateway.dev.ext_mcp.McpResponseResult.mutated",
			".agentgateway.dev.ext_mcp.AuthorizationError.mcp_error",
		]);
		c.extern_path(".google.protobuf.Value", "::prost_wkt_types::Value");
		c.extern_path(".google.protobuf.Struct", "::prost_wkt_types::Struct");
		c
	};

	let fds = protox::compile(&proto_files, &include_dirs)?;
	tonic_prost_build::configure()
		.build_server(true)
		.compile_fds_with_config(fds, config)?;

	for path in [proto_files, include_dirs.to_vec()].concat() {
		println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
	}

	Ok(())
}
