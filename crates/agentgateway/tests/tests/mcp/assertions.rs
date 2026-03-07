use anyhow::ensure;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::time::{Duration, Instant};

pub(super) fn assert_prefixed_by(name: &str, target: &str) {
	assert!(
		name.starts_with(&format!("{target}__")),
		"expected '{name}' to be prefixed by '{target}__'"
	);
}

pub(super) fn assert_wrapped_uri_for_target(uri: &str, target: &str) {
	assert!(
		uri.starts_with(&format!("agw://{target}/")),
		"expected URI '{uri}' to be wrapped for target '{target}'"
	);
}

pub(super) async fn wait_for_resource_updates(update_count: &AtomicUsize) -> anyhow::Result<()> {
	let deadline = Instant::now() + Duration::from_secs(5);
	while update_count.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
		tokio::time::sleep(Duration::from_millis(50)).await;
	}
	ensure!(
		update_count.load(Ordering::SeqCst) > 0,
		"expected at least one resources/updated notification after subscribe"
	);
	Ok(())
}
