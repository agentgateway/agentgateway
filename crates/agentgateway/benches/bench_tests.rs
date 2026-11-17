use divan::Bencher;
// #[global_allocator]
// static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
	eprintln!("Benchmarking...");
	#[cfg(all(not(test), not(feature = "internal_benches")))]
	panic!("benches must have -F internal_benches");
	#[allow(unused)]
	use agentgateway as _;
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

// Sampling slows the code under test; set AGENTGATEWAY_BENCH_NO_PPROF=1 for clean timings.
#[cfg(target_family = "unix")]
pub fn with_profiling(f: impl FnOnce()) {
	use std::fs::File;
	use std::io::Write;

	use pprof::protos::Message;
	if std::env::var_os("AGENTGATEWAY_BENCH_NO_PPROF").is_some_and(|v| !v.is_empty()) {
		return f();
	}

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
