mod cel;
mod metrics;
mod schema;
mod tracing;

use std::env::args;

use anyhow::{Context, Result, bail};

enum Task {
	Schema,
	Metrics,
	Tracing,
	Cel,
}

fn get_task() -> Result<Task> {
	let message = "argument is missing. Example usage: \ncargo xtask <schema|metrics|tracing|cel>";
	let arg = args().nth(1).context(message)?;
	match arg.as_str() {
		"schema" => Ok(Task::Schema),
		"metrics" => Ok(Task::Metrics),
		"tracing" => Ok(Task::Tracing),
		"cel" => Ok(Task::Cel),
		arg => bail!("unknown task: {}", arg),
	}
}

fn main() -> Result<()> {
	match get_task()? {
		Task::Schema => schema::generate_schema(),
		Task::Metrics => metrics::generate_metrics(),
		Task::Tracing => tracing::generate_tracing(),
		Task::Cel => cel::evaluate_command(),
	}
}
