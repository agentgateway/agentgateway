#![no_main]

use agentgateway::cel::{ExecutorSerde, Expression, full_example_executor};
use libfuzzer_sys::fuzz_target;
use once_cell::sync::Lazy;

static EXECUTOR: Lazy<ExecutorSerde> = Lazy::new(full_example_executor);

fuzz_target!(|data: &[u8]| {
	if data.len() > 16 * 1024 {
		return;
	}
	let Ok(expr) = std::str::from_utf8(data) else {
		return;
	};

	if let Ok(expr) = Expression::new_strict(expr) {
		let exec = EXECUTOR.as_executor();
		if let Ok(value) = exec.eval(&expr) {
			let _ = value.json();
		}
	}
});
