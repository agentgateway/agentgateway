use std::path::Path;

fn main() {
	println!("cargo::rustc-check-cfg=cfg(ui_assets)");
	let dist = Path::new("../../ui/dist");
	if std::env::var_os("CARGO_FEATURE_UI").is_some() || dist.join("index.html").exists() {
		// With the `ui` feature and no assets, include_dir! fails the build.
		println!("cargo::rustc-cfg=ui_assets");
		println!("cargo::rerun-if-changed={}", dist.display());
	} else {
		// Watching a missing path would rerun this (and relink) on every build, so a new
		// ui/dist is only picked up once something else in this crate changes.
		println!("cargo::rerun-if-changed=build.rs");
		println!("cargo::warning=ui/dist not found; building without the UI");
	}
}
