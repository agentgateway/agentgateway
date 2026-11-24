use std::fmt::Write;

use agentgateway::telemetry::trace_attributes::{RequestKind, span_attributes};
use anyhow::Result;

pub fn generate_tracing() -> Result<()> {
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()?;
	let attributes = runtime.block_on(span_attributes())?;

	let mut markdown = String::from(
		"# Trace span attributes\n\nAttributes on the request span. An attribute is set only when the request has a value for it.\n",
	);
	for kind in [
		RequestKind::Common,
		RequestKind::Llm,
		RequestKind::Mcp,
		RequestKind::A2a,
		RequestKind::Substrate,
	] {
		let group: Vec<_> = attributes.iter().filter(|a| a.kind == kind).collect();
		if group.is_empty() {
			continue;
		}
		writeln!(
			markdown,
			"\n## {}\n\n| Attribute | Condition |\n| --- | --- |",
			kind.title()
		)?;
		for attribute in group {
			writeln!(
				markdown,
				"| `{}` | {} |",
				attribute.name,
				attribute.condition.unwrap_or("—")
			)?;
		}
	}

	let xtask_path = std::env::var("CARGO_MANIFEST_DIR")?;
	fs_err::write(format!("{xtask_path}/../../schema/tracing.md"), markdown)?;
	Ok(())
}
