use std::env;

use agentgateway::cel;

pub fn evaluate_command() -> anyhow::Result<()> {
	let mut args: Vec<String> = env::args().collect();

	let ast = args.get(2).is_some_and(|arg| arg == "--ast");
	if ast {
		args.remove(2);
	}

	if args.len() != 3 && (!ast && args.len() != 4) {
		anyhow::bail!(
			"Usage: {} cel [--ast] <expression> [<json>]",
			args.first().map(|s| s.as_str()).unwrap_or("xtask")
		);
	}
	let expression = &args[2];
	if ast {
		let expr = ::cel::Program::compile_unoptimized(expression)?;
		let js = serde_json::to_string_pretty(expr.expression())?;
		println!("{}", js);
		return Ok(());
	}

	let expr = cel::Expression::new_strict(expression)?;
	let v = if args.len() == 4 {
		let json_input = &args[3];
		let v: cel::ExecutorSerde = serde_json::from_str(json_input)?;
		v
	} else {
		cel::full_example_executor()
	};

	let exec = v.as_executor();
	let res = exec.eval(&expr)?;
	let j = res
		.json()
		.map_err(|e| anyhow::anyhow!("failed to serialize result: {}", e))?;
	let js = serde_json::to_string_pretty(&j)?;
	println!("{}", js);
	Ok(())
}
