use std::fs::File;
use std::io::Write;

use divan::Bencher;
// #[global_allocator]
// static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
	eprintln!("Benchmarking...");
	#[cfg(all(not(test), not(feature = "internal_benches")))]
	panic!("benches must have -F internal_benches");
	with_profiling(divan::main);
}
#[divan::bench()]
fn bench(b: Bencher) {
	b.bench(|| {});
}

#[cfg(not(target_family = "unix"))]
pub fn with_profiling(f: impl FnOnce()) {
	f()
}

#[cfg(target_family = "unix")]
pub fn with_profiling(f: impl FnOnce()) {
	use pprof::protos::Message;
	let guard = pprof::ProfilerGuardBuilder::default()
		.frequency(1000)
		.build()
		.unwrap();

	f();
	eprintln!("Writing profile to /tmp/pprof-agentgateway.prof...");

	let report = guard.report().build().unwrap();
	let profile = report.pprof().unwrap();

	let body = profile.write_to_bytes().unwrap();
	File::create("/tmp/pprof-agentgateway.prof")
		.unwrap()
		.write_all(&body)
		.unwrap()
}
