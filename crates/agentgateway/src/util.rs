use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf, absolute};
use std::time::Duration;

use notify::{EventKind, RecursiveMode};
use tokio::sync::mpsc;
use tracing::warn;

pub fn is_runtime_shutdown(e: &Error) -> bool {
	if e.kind() == ErrorKind::Other
		&& e.to_string() == "A Tokio 1.x context was found, but it is being shutdown."
	{
		return true;
	}
	false
}

/// Watches `paths` and their parent directories, emitting `()` on the returned
/// receiver once per debounced batch of real content changes.
pub struct WatchedFiles {
	pub paths: Vec<PathBuf>,
	pub changes: mpsc::Receiver<()>,
}

pub fn watch_files(paths: Vec<PathBuf>) -> anyhow::Result<WatchedFiles> {
	let (raw_tx, mut raw_rx) = mpsc::channel(1);
	let mut watcher =
		notify_debouncer_full::new_debouncer(Duration::from_millis(250), None, move |res| {
			futures::executor::block_on(async {
				let _ = raw_tx.send(res).await;
			})
		})
		.map_err(|e| anyhow::anyhow!("failed to create file watcher: {e}"))?;

	let abspaths = paths
		.iter()
		.map(absolute)
		.collect::<std::io::Result<Vec<_>>>()?;
	if abspaths.is_empty() {
		anyhow::bail!("no files supplied to watch");
	}

	let mut watched_targets: Vec<PathBuf> = Vec::new();
	let mut watch_errors = Vec::new();
	let mut unwatched_paths = Vec::new();
	for abspath in &abspaths {
		let parent = abspath.parent().ok_or_else(|| {
			anyhow::anyhow!(
				"failed to get the parent of watched file {}",
				abspath.display()
			)
		})?;
		let mut path_watched = false;
		for target in [abspath.as_path(), parent] {
			if watched_targets.iter().any(|p| p == target) {
				path_watched = true;
				continue;
			}
			match watcher.watch(target, RecursiveMode::NonRecursive) {
				Ok(()) => {
					watched_targets.push(target.to_path_buf());
					path_watched = true;
				},
				Err(e) => {
					watch_errors.push(format!("{}: {}", target.display(), e));
					warn!("failed to watch path {}: {}", target.display(), e);
				},
			}
		}
		if !path_watched {
			unwatched_paths.push(abspath.display().to_string());
		}
	}
	if !unwatched_paths.is_empty() {
		return Err(anyhow::anyhow!(
			"failed to watch configured file paths: {}; watch errors: {}",
			unwatched_paths.join(", "),
			watch_errors.join(", ")
		));
	}

	let (change_tx, change_rx) = mpsc::channel(1);
	let watched_paths = abspaths.clone();
	let mut targets = resolve_targets(&abspaths);
	tokio::task::spawn(async move {
		while let Some(events) = raw_rx.recv().await {
			match events {
				Ok(events) => {
					let current = resolve_targets(&abspaths);
					let triggered =
						batch_triggers_reload(events.iter().map(|e| &**e), &abspaths, &targets, &current);
					targets = current;
					if triggered && change_tx.send(()).await.is_err() {
						break;
					}
				},
				Err(errors) => warn!("file watch error: {errors:?}"),
			}
		}
		drop(watcher);
	});
	Ok(WatchedFiles {
		paths: watched_paths,
		changes: change_rx,
	})
}

fn resolve_targets(paths: &[PathBuf]) -> Vec<Option<PathBuf>> {
	paths.iter().map(|path| resolve_symlink(path)).collect()
}

/// Resolves a symlink to its final target, returns the path itself when it is a
/// regular file, or `None` when the path is missing or unreadable.
fn resolve_symlink(path: &Path) -> Option<PathBuf> {
	match fs_err::symlink_metadata(path) {
		Ok(meta) if meta.file_type().is_symlink() => fs_err::canonicalize(path).ok(),
		Ok(_) => Some(path.to_path_buf()),
		Err(_) => None,
	}
}

fn batch_triggers_reload<'a>(
	events: impl IntoIterator<Item = &'a notify::Event>,
	abspaths: &[PathBuf],
	previous_targets: &[Option<PathBuf>],
	current_targets: &[Option<PathBuf>],
) -> bool {
	// A target appearing or changing means a valid new version is available. A
	// disappearance resolves to `None`; ignore it and keep the last good catalog.
	let target_rotated =
		previous_targets
			.iter()
			.zip(current_targets.iter())
			.any(|(previous, current)| match (previous, current) {
				(None, Some(_)) => true,
				(Some(previous), Some(current)) => previous != current,
				_ => false,
			});
	target_rotated
		|| events
			.into_iter()
			.any(|event| should_reload(event, abspaths, current_targets))
}

fn should_reload(event: &notify::Event, abspaths: &[PathBuf], targets: &[Option<PathBuf>]) -> bool {
	if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
		return false;
	}
	event.paths.iter().any(|path| {
		abspaths.iter().any(|abspath| abspath == path)
			|| targets.iter().any(|t| t.as_deref() == Some(path.as_path()))
	})
}

#[cfg(test)]
mod tests {
	use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind};

	use super::*;

	fn modify(path: &str) -> notify::Event {
		notify::Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Any)))
			.add_path(PathBuf::from(path))
	}

	fn open(path: &str) -> notify::Event {
		notify::Event::new(EventKind::Access(AccessKind::Open(AccessMode::Any)))
			.add_path(PathBuf::from(path))
	}

	#[test]
	fn reloads_on_modify_of_watched_file() {
		let file = PathBuf::from("/cfg/price.json");
		let target = file.clone();
		let event = modify("/cfg/price.json");
		assert!(should_reload(
			&event,
			std::slice::from_ref(&file),
			&[Some(target)]
		));
	}

	#[test]
	fn reloads_on_modify_of_resolved_symlink_target() {
		let file = PathBuf::from("/cfg/price.json");
		let target = PathBuf::from("/cfg/..data/price.json");
		let event = modify("/cfg/..data/price.json");
		assert!(should_reload(&event, &[file], &[Some(target)]));
	}

	#[test]
	fn ignores_access_open_events() {
		// Guards the self-trigger loop: re-reading the file emits OPEN events.
		let file = PathBuf::from("/cfg/price.json");
		let target = file.clone();
		let event = open("/cfg/price.json");
		assert!(!should_reload(
			&event,
			std::slice::from_ref(&file),
			&[Some(target)]
		));
	}

	#[test]
	fn ignores_unrelated_sibling_writes() {
		let file = PathBuf::from("/cfg/price.json");
		let target = file.clone();
		let event = modify("/cfg/runc-process35026236");
		assert!(!should_reload(
			&event,
			std::slice::from_ref(&file),
			&[Some(target)]
		));
	}

	#[test]
	fn reloads_on_symlink_target_change_without_matching_event_path() {
		let file = PathBuf::from("/cfg/price.json");
		let old = vec![Some(PathBuf::from("/cfg/..2024/price.json"))];
		let new = vec![Some(PathBuf::from("/cfg/..2025/price.json"))];
		// Create event on the parent only — no path matches the watched file.
		let event =
			notify::Event::new(EventKind::Create(CreateKind::Any)).add_path(PathBuf::from("/cfg/..data"));
		assert!(batch_triggers_reload([&event], &[file], &old, &new));
	}

	#[test]
	fn no_reload_when_file_disappears() {
		// A delete resolves the target to None; keep the last good catalog rather
		// than reloading against a missing file.
		let file = PathBuf::from("/cfg/price.json");
		let old = vec![Some(PathBuf::from("/cfg/price.json"))];
		let new = vec![None];
		let event = notify::Event::new(EventKind::Remove(notify::event::RemoveKind::File))
			.add_path(PathBuf::from("/cfg/price.json"));
		assert!(!batch_triggers_reload([&event], &[file], &old, &new));
	}

	#[test]
	fn no_reload_when_one_of_multiple_files_disappears() {
		let files = vec![
			PathBuf::from("/cfg/price.json"),
			PathBuf::from("/cfg/override.json"),
		];
		let old = vec![
			Some(PathBuf::from("/cfg/price.json")),
			Some(PathBuf::from("/cfg/override.json")),
		];
		let new = vec![None, Some(PathBuf::from("/cfg/override.json"))];
		let event = notify::Event::new(EventKind::Remove(notify::event::RemoveKind::File))
			.add_path(PathBuf::from("/cfg/price.json"));
		assert!(!batch_triggers_reload([&event], &files, &old, &new));
	}

	#[test]
	fn reloads_when_missing_file_appears_without_matching_event_path() {
		let file = PathBuf::from("/cfg/price.json");
		let old = vec![None];
		let new = vec![Some(PathBuf::from("/cfg/price.json"))];
		let event =
			notify::Event::new(EventKind::Create(CreateKind::Any)).add_path(PathBuf::from("/cfg"));
		assert!(batch_triggers_reload([&event], &[file], &old, &new));
	}

	#[test]
	fn no_reload_when_only_opens_and_nothing_changed() {
		let file = PathBuf::from("/cfg/price.json");
		let targets = vec![Some(PathBuf::from("/cfg/price.json"))];
		let event = open("/cfg/price.json");
		assert!(!batch_triggers_reload(
			[&event],
			&[file],
			&targets,
			&targets
		));
	}

	#[cfg(target_family = "unix")]
	#[tokio::test]
	async fn live_watcher_reports_symlink_rotation() {
		use std::os::unix::fs::symlink;

		let dir = tempfile::tempdir().unwrap();
		let v1 = dir.path().join("..2024");
		let v2 = dir.path().join("..2025");
		fs_err::tokio::create_dir(&v1).await.unwrap();
		fs_err::tokio::write(v1.join("price.json"), "{}")
			.await
			.unwrap();
		symlink("..2024", dir.path().join("..data")).unwrap();
		symlink("..data/price.json", dir.path().join("price.json")).unwrap();

		let mut watched = watch_files(vec![dir.path().join("price.json")]).unwrap();

		fs_err::tokio::create_dir(&v2).await.unwrap();
		fs_err::tokio::write(v2.join("price.json"), "{\"version\":2}")
			.await
			.unwrap();
		fs_err::remove_file(dir.path().join("..data")).unwrap();
		symlink("..2025", dir.path().join("..data")).unwrap();

		tokio::time::timeout(Duration::from_secs(5), watched.changes.recv())
			.await
			.expect("watcher should report a symlink rotation")
			.expect("watcher channel should remain open");
	}
}
