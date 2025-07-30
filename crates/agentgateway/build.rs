// This build script is used to generate the rust source files that
// we need for XDS GRPC communication.
fn main() -> Result<(), anyhow::Error> {
	let proto_files = [
		"proto/ext_proc.proto",
		"proto/ext_authz.proto",
		"proto/rls.proto",
		"proto/resource.proto",
		"proto/workload.proto",
		"proto/citadel.proto",
	]
	.iter()
	.map(|name| std::env::current_dir().unwrap().join(name))
	.collect::<Vec<_>>();
	let include_dirs = ["proto/"]
		.iter()
		.map(|i| std::env::current_dir().unwrap().join(i))
		.collect::<Vec<_>>();
	let config = {
		let mut c = prost_build::Config::new();
		c.disable_comments(Some("."));
		c.bytes([
			".agentgateway.dev.workload.Workload",
			".agentgateway.dev.workload.Service",
			".agentgateway.dev.workload.GatewayAddress",
			".agentgateway.dev.workload.Address",
			".agentgateway.dev.workload.Address",
		]);
		c
	};
	tonic_prost_build::configure()
		.build_server(true)
		.compile_with_config(
			config,
			&proto_files
				.iter()
				.map(|path| path.to_str().unwrap())
				.collect::<Vec<_>>(),
			&include_dirs
				.iter()
				.map(|p| p.to_str().unwrap())
				.collect::<Vec<_>>(),
		)?;

	// This tells cargo to re-run this build script only when the proto files
	// we're interested in change or the any of the proto directories were updated.
	for path in [proto_files, include_dirs].concat() {
		println!("cargo:rerun-if-changed={}", path.to_str().unwrap());
	}
	Ok(())
}
