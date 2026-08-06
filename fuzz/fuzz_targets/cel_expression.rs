#![no_main]

use agent_celx::DefaultOptimizer;
use cel::context::MapResolver;
use cel::{Context, Program};
use libfuzzer_sys::fuzz_target;
use serde_json::json;

fuzz_target!(|data: &[u8]| {
	if data.len() > 16 * 1024 {
		return;
	}
	let Ok(expr) = std::str::from_utf8(data) else {
		return;
	};

	if let Ok(program) = Program::compile_with_optimizer(expr, DefaultOptimizer) {
		let mut context = Context::default();
		agent_celx::insert_all(&mut context);
		let data = json!({
			"request": {"path": "/api/test", "headers": {"user-agent": "example"}},
			"response": {"code": 200},
			"source": {"address": "127.0.0.1"},
			"llm": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15},
			"mcp": {"tool": {"name": "echo"}},
		});
		let mut variables = MapResolver::new();
		for (name, value) in data.as_object().expect("static object") {
			variables.add_variable(name, value).expect("static value");
		}
		if let Ok(value) = program.execute_with(&context, &variables) {
			let _ = value.json();
		}
	}
});
