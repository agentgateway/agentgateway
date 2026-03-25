fn main() -> Result<(), anyhow::Error> {
	let cwd = std::env::current_dir()?;
	let proto_files = [
		"proto/shared_envoy.proto",
		"proto/xds.proto",
		"proto/citadel.proto",
		"proto/ext_authz.proto",
		"proto/ext_proc.proto",
		"proto/rls.proto",
		"proto/workload.proto",
		"proto/resource.proto",
	]
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
