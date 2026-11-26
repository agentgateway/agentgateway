use agentgateway::management::readiness_server;

use crate::ProbeArgs;

pub(crate) fn execute(args: ProbeArgs) -> anyhow::Result<()> {
	tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()?
		.block_on(readiness_server::probe(args.address))
}
