//! Envoy-style overload guard: report how close the process cgroup is to its memory limit so
//! the accept loop can stop taking work before the kernel OOM killer steps in.
//!
//! Two details matter for correctness:
//!
//! * The comparison uses the *working set* (`memory.current` minus reclaimable page cache),
//!   the same quantity kubelet uses. Raw `memory.current` counts page cache, so a container
//!   that merely read files can sit at 90% of its limit while nowhere near an OOM — using it
//!   would make the guard stop serving traffic permanently.
//! * Values are sampled by a background task, never from the accept loop. Reading sysfs is a
//!   blocking syscall, and the accept loop is a single task shared by every connection.
//!
//! Sampling only starts once a bind opts in, so gateways without the setting keep their
//! previous behaviour and never touch the filesystem.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

const V2_CURRENT: &str = "/sys/fs/cgroup/memory.current";
const V2_MAX: &str = "/sys/fs/cgroup/memory.max";
const V2_STAT: &str = "/sys/fs/cgroup/memory.stat";
const V1_USAGE: &str = "/sys/fs/cgroup/memory/memory.usage_in_bytes";
const V1_LIMIT: &str = "/sys/fs/cgroup/memory/memory.limit_in_bytes";
const V1_STAT: &str = "/sys/fs/cgroup/memory/memory.stat";

/// Working set as a percentage of the cgroup limit. `0` also means "unknown", which keeps the
/// guard fail-open when the cgroup is unreadable or unlimited.
static USAGE_PERCENT: AtomicU8 = AtomicU8::new(0);
static WATCHER: OnceLock<()> = OnceLock::new();

/// Latest sampled working set percentage. Cheap enough for the accept loop.
pub(crate) fn working_set_percent() -> u8 {
	USAGE_PERCENT.load(Ordering::Relaxed)
}

/// Start the sampler once, the first time any bind configures a threshold.
pub(crate) fn ensure_sampler() {
	WATCHER.get_or_init(|| {
		// Seed synchronously so the first accept decision is not made on a stale zero.
		USAGE_PERCENT.store(sample().unwrap_or(0), Ordering::Relaxed);
		tokio::spawn(async {
			let mut ticker = tokio::time::interval(SAMPLE_INTERVAL);
			ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
			loop {
				ticker.tick().await;
				let pct = tokio::task::spawn_blocking(sample)
					.await
					.ok()
					.flatten()
					.unwrap_or(0);
				USAGE_PERCENT.store(pct, Ordering::Relaxed);
			}
		});
	});
}

fn sample() -> Option<u8> {
	let (working_set, limit) = read_working_set()?;
	if limit == 0 {
		return None;
	}
	Some((working_set.saturating_mul(100) / limit).min(100) as u8)
}

fn read_working_set() -> Option<(u64, u64)> {
	read_v2().or_else(read_v1)
}

fn read_v2() -> Option<(u64, u64)> {
	let current = read_u64(V2_CURRENT)?;
	let limit = read_limit(V2_MAX)?;
	let inactive_file = read_stat_field(V2_STAT, "inactive_file").unwrap_or(0);
	Some((current.saturating_sub(inactive_file), limit))
}

fn read_v1() -> Option<(u64, u64)> {
	let usage = read_u64(V1_USAGE)?;
	let limit = read_limit(V1_LIMIT)?;
	let inactive_file = read_stat_field(V1_STAT, "total_inactive_file").unwrap_or(0);
	Some((usage.saturating_sub(inactive_file), limit))
}

fn read_u64(path: &str) -> Option<u64> {
	std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_limit(path: &str) -> Option<u64> {
	let raw = std::fs::read_to_string(path).ok()?;
	let raw = raw.trim();
	if raw.eq_ignore_ascii_case("max") {
		return None;
	}
	let limit: u64 = raw.parse().ok()?;
	// cgroup v1 spells "unlimited" as a huge page-aligned sentinel.
	(limit < (1u64 << 62)).then_some(limit)
}

fn read_stat_field(path: &str, field: &str) -> Option<u64> {
	let stat = std::fs::read_to_string(path).ok()?;
	parse_stat_field(&stat, field)
}

fn parse_stat_field(stat: &str, field: &str) -> Option<u64> {
	stat.lines().find_map(|line| {
		let (key, value) = line.split_once(' ')?;
		(key == field).then(|| value.trim().parse().ok())?
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_requested_stat_field_only() {
		let stat = "anon 1048576\nfile 2097152\ninactive_file 1572864\nslab 4096\n";
		assert_eq!(parse_stat_field(stat, "inactive_file"), Some(1572864));
		assert_eq!(parse_stat_field(stat, "anon"), Some(1048576));
		assert_eq!(parse_stat_field(stat, "missing"), None);
	}

	#[test]
	fn ignores_prefix_collisions() {
		let stat = "inactive_file_extra 5\ninactive_file 7\n";
		assert_eq!(parse_stat_field(stat, "inactive_file"), Some(7));
	}

	#[test]
	fn unreadable_cgroup_reports_unknown() {
		assert_eq!(read_u64("/nonexistent/agentgateway/memory.current"), None);
		assert_eq!(read_limit("/nonexistent/agentgateway/memory.max"), None);
	}

	#[test]
	fn sampler_is_disabled_until_requested() {
		// No bind opted in during this test binary's unit tests, so the guard stays fail-open.
		assert_eq!(working_set_percent(), 0);
	}
}
