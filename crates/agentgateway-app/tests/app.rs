#[test]
fn test_compiles() {
	let _ = agentgateway_app::run as fn() -> anyhow::Result<()>;
}
