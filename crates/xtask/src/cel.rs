use std::env;

use agentgateway::cel;

pub fn evaluate_command() -> anyhow::Result<()> {
	let args: Vec<String> = env::args().collect();

	if args.len() != 3 && args.len() != 4 {
		anyhow::bail!(
			"Usage: {} <expression> [<json>]",
			args.first().map(|s| s.as_str()).unwrap_or("xtask")
		);
	}
	let expression = &args[2];
	let v = if args.len() == 4 {
		let json_input = &args[3];
		let v: cel::ExecutorSerde = serde_json::from_str(json_input)?;
		v
	} else {
		cel::full_example_executor()
	};

	let expr = cel::Expression::new_strict(expression)?;
	let exec = v.as_executor();
	let res = exec.eval(&expr)?;
	let j = res
		.json()
		.map_err(|e| anyhow::anyhow!("failed to serialize result: {}", e))?;
	let js = serde_json::to_string_pretty(&j)?;
	println!("{}", js);
	Ok(())
}
