#[test]
fn expand() {
	use std::{env, fs};

	let path_var = env::var_os("PATH").unwrap_or_default();
	let mut paths = env::split_paths(&path_var);
	let found = paths.any(|dir| {
		let exe = dir.join(if cfg!(windows) {
			"cargo-expand.exe"
		} else {
			"cargo-expand"
		});
		fs::metadata(exe).is_ok()
	});

	if !found {
		eprintln!("cargo-expand not found in PATH, skipping expand test.");
		return;
	}

	macrotest::expand("tests/expand/*.rs");
}
