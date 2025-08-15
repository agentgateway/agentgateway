use std::env;
use std::process::Command;

fn main() {
	let out_dir = env::var("OUT_DIR").unwrap();
	// Adopted from https://github.com/uutils/coreutils/blob/main/src/uu/stdbuf/build.rs
	let profile_name = out_dir
		.split(std::path::MAIN_SEPARATOR)
		.nth_back(3)
		.unwrap();
	let target = env::var("TARGET").unwrap();

	let output = if cfg!(target_os = "windows") {
		Command::new("powershell.exe")
			.arg("../../common/scripts/report_build_info.ps1")
			.output()
	} else {
		Command::new("../../common/scripts/report_build_info.sh").output()
	};

	match output {
		Ok(output) => {
			for line in String::from_utf8(output.stdout).unwrap().lines() {
				// Each line looks like `agentgateway.dev.buildGitRevision=abc`
				if let Some((key, value)) = line.split_once('=') {
					#[allow(clippy::double_ended_iterator_last)]
					let key = key.split('.').last().unwrap();
					println!("cargo:rustc-env=AGENTGATEWAY_BUILD_{key}={value}");
				} else {
					println!("cargo:warning=invalid build output {line}");
				}
			}
		},
		Err(err) => {
			println!("cargo:warning={err}");
		},
	};
	println!(
		"cargo:rustc-env=AGENTGATEWAY_BUILD_RUSTC_VERSION={}",
		rustc_version::version().unwrap()
	);
	println!("cargo:rustc-env=AGENTGATEWAY_BUILD_PROFILE_NAME={profile_name}");
	println!("cargo:rustc-env=AGENTGATEWAY_BUILD_TARGET={target}");
	println!("cargo:rerun-if-env-changed=VERSION");
}
