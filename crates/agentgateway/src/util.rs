use std::io::{Error, ErrorKind};
use std::path::{Component, Path, PathBuf, absolute};
use std::time::Duration;

use anyhow::{anyhow, bail};
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

pub struct WatchedFiles {
	paths: Vec<PathBuf>,
	changes: mpsc::Receiver<()>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WatchFilesOptions {
	reload_on_disappearance: bool,
}

impl WatchFilesOptions {
	pub fn reload_on_disappearance(mut self, reload: bool) -> Self {
		self.reload_on_disappearance = reload;
		self
	}
}

impl WatchedFiles {
	pub fn paths(&self) -> &[PathBuf] {
		&self.paths
	}
	pub async fn changed(&mut self) -> bool {
		self.changes.recv().await.is_some()
	}
}

pub fn watch_files(paths: Vec<PathBuf>) -> anyhow::Result<WatchedFiles> {
	watch_files_with_options(paths, WatchFilesOptions::default())
}

pub fn watch_files_with_options(
	paths: Vec<PathBuf>,
	options: WatchFilesOptions,
) -> anyhow::Result<WatchedFiles> {
	let (raw_tx, mut raw_rx) = mpsc::channel(1);

	let mut watcher =
		notify_debouncer_full::new_debouncer(Duration::from_millis(250), None, move |res| {
			futures::executor::block_on(async {
				let _ = raw_tx.send(res).await;
			})
		})
		.map_err(|e| anyhow!("failed to create file watcher: {e}"))?;

	let paths = paths
		.iter()
		.map(absolute)
		.collect::<std::io::Result<Vec<_>>>()?;
	if paths.is_empty() {
		bail!("no files supplied to watch");
	}

	let mut watched_targets: Vec<PathBuf> = Vec::new();
	let mut watch_errors = Vec::new();
	let mut unwatched_paths = Vec::new();
	for p in &paths {
		let mut path_watched = false;
		for target in watch_targets_for_path(p)? {
			if watched_targets.iter().any(|p| p == &target) {
				path_watched = true;
				continue;
			}
			match watcher.watch(&target, RecursiveMode::NonRecursive) {
				Ok(()) => {
					watched_targets.push(target.clone());
					path_watched = true;
				},
				Err(e) => {
					let reason = notify_error_reason(&e);
					watch_errors.push(format!("{}: {}", target.display(), reason));
					warn!("failed to watch path {}: {}", target.display(), reason);
				},
			}
		}
		if !path_watched {
			unwatched_paths.push(p.display().to_string());
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
	let watched_paths = paths.clone();
	let mut targets = resolve_targets(&paths);
	tokio::task::spawn(async move {
		while let Some(events) = raw_rx.recv().await {
			match events {
				Ok(events) => {
					let current = resolve_targets(&paths);
					let triggered = batch_triggers_reload(
						events.iter().map(|e| &**e),
						&paths,
						&targets,
						&current,
						options,
					);
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

fn watch_targets_for_path(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
	let mut targets = vec![path.to_path_buf()];
	if requires_parent_watch(path)? {
		let parent = path.parent().ok_or_else(|| {
			anyhow!(
				"failed to get the parent of watched file {}",
				path.display()
			)
		})?;
		targets.push(parent.to_path_buf());
	}
	Ok(targets)
}

fn requires_parent_watch(path: &Path) -> anyhow::Result<bool> {
	let meta = match fs_err::symlink_metadata(path) {
		Ok(meta) => meta,
		Err(e) if e.kind() == ErrorKind::NotFound => return Ok(true),
		Err(e) => return Err(e.into()),
	};
	Ok(meta.file_type().is_symlink() && is_kubernetes_projected_volume_symlink(path))
}

fn is_kubernetes_projected_volume_symlink(path: &Path) -> bool {
	let Some(parent) = path.parent() else {
		return false;
	};
	let Ok(link) = fs_err::read_link(path) else {
		return false;
	};
	if has_kubernetes_projection_component(&link) {
		return true;
	}
	let target = if link.is_absolute() {
		link
	} else {
		parent.join(link)
	};
	let Ok(target) = fs_err::canonicalize(target) else {
		return false;
	};
	target
		.strip_prefix(parent)
		.is_ok_and(has_kubernetes_projection_component)
}

fn has_kubernetes_projection_component(path: &Path) -> bool {
	path.components().any(|component| {
		let Component::Normal(component) = component else {
			return false;
		};
		component
			.to_str()
			.is_some_and(|name| name == "..data" || name.starts_with("..20"))
	})
}

fn notify_error_reason(e: &notify::Error) -> String {
	match &e.kind {
		notify::ErrorKind::Generic(err) => err.clone(),
		notify::ErrorKind::Io(err) => err.to_string(),
		notify::ErrorKind::PathNotFound => "No path was found.".to_string(),
		notify::ErrorKind::WatchNotFound => "No watch was found.".to_string(),
		notify::ErrorKind::InvalidConfig(config) => format!("Invalid configuration: {config:?}"),
		notify::ErrorKind::MaxFilesWatch => "OS file watch limit reached.".to_string(),
	}
}

fn resolve_targets(paths: &[PathBuf]) -> Vec<Option<PathBuf>> {
	paths
		.iter()
		.map(|p| match fs_err::symlink_metadata(p) {
			Ok(meta) if meta.file_type().is_symlink() => fs_err::canonicalize(p).ok(),
			Ok(_) => Some(p.clone()),
			Err(_) => None,
		})
		.collect()
}

fn batch_triggers_reload<'a>(
	events: impl IntoIterator<Item = &'a notify::Event>,
	abspaths: &[PathBuf],
	previous_targets: &[Option<PathBuf>],
	current_targets: &[Option<PathBuf>],
	options: WatchFilesOptions,
) -> bool {
	// A target appearing or changing means a valid new version is available.
	// Disappearance is opt-in because some callers should keep last-good state.
	let target_rotated =
		previous_targets
			.iter()
			.zip(current_targets.iter())
			.any(|(previous, current)| match (previous, current) {
				(None, Some(_)) => true,
				(Some(previous), Some(current)) => previous != current,
				(Some(_), None) => options.reload_on_disappearance,
				_ => false,
			});
	target_rotated
		|| events
			.into_iter()
			.any(|event| should_reload(event, abspaths, current_targets))
}

fn should_reload(event: &notify::Event, abspaths: &[PathBuf], targets: &[Option<PathBuf>]) -> bool {
	if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
		return event.paths.iter().any(|path| {
			abspaths.iter().any(|abspath| abspath == path)
				|| targets.iter().any(|t| t.as_deref() == Some(path.as_path()))
		});
	}
	false
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
	fn notify_error_reason_omits_notify_paths() {
		let err = notify::Error::path_not_found().add_path(PathBuf::from("/cfg/override.json"));
		assert_eq!(notify_error_reason(&err), "No path was found.");
	}

	#[test]
	fn regular_file_watches_only_the_file() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("config.yaml");
		fs_err::write(&file, "{}").unwrap();
		assert_eq!(watch_targets_for_path(&file).unwrap(), vec![file]);
	}

	#[test]
	fn missing_file_watches_parent_for_appearance() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("catalog.json");
		assert_eq!(
			watch_targets_for_path(&file).unwrap(),
			vec![file.clone(), dir.path().to_path_buf()]
		);
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn kubernetes_projected_symlink_watches_file_and_parent() {
		use std::os::unix::fs::symlink;

		let dir = tempfile::tempdir().unwrap();
		let version = dir.path().join("..2024_01_01_00_00_00.000000000");
		fs_err::create_dir(&version).unwrap();
		fs_err::write(version.join("config.yaml"), "{}").unwrap();
		symlink("..2024_01_01_00_00_00.000000000", dir.path().join("..data")).unwrap();
		let file = dir.path().join("config.yaml");
		symlink("..data/config.yaml", &file).unwrap();

		assert_eq!(
			watch_targets_for_path(&file).unwrap(),
			vec![file.clone(), dir.path().to_path_buf()]
		);
	}

	#[cfg(target_family = "unix")]
	#[test]
	fn ordinary_symlink_watches_only_the_file() {
		use std::os::unix::fs::symlink;

		let dir = tempfile::tempdir().unwrap();
		let real = dir.path().join("real");
		fs_err::create_dir(&real).unwrap();
		fs_err::write(real.join("config.yaml"), "{}").unwrap();
		let file = dir.path().join("config.yaml");
		symlink("real/config.yaml", &file).unwrap();

		assert_eq!(watch_targets_for_path(&file).unwrap(), vec![file]);
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
		assert!(batch_triggers_reload(
			[&event],
			&[file],
			&old,
			&new,
			WatchFilesOptions::default()
		));
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
		assert!(!batch_triggers_reload(
			[&event],
			&[file],
			&old,
			&new,
			WatchFilesOptions::default()
		));
	}

	#[test]
	fn reloads_when_disappearance_is_enabled() {
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
		assert!(batch_triggers_reload(
			[&event],
			&files,
			&old,
			&new,
			WatchFilesOptions::default().reload_on_disappearance(true)
		));
	}

	#[test]
	fn reloads_when_missing_file_appears_without_matching_event_path() {
		let file = PathBuf::from("/cfg/price.json");
		let old = vec![None];
		let new = vec![Some(PathBuf::from("/cfg/price.json"))];
		let event =
			notify::Event::new(EventKind::Create(CreateKind::Any)).add_path(PathBuf::from("/cfg"));
		assert!(batch_triggers_reload(
			[&event],
			&[file],
			&old,
			&new,
			WatchFilesOptions::default()
		));
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
			&targets,
			WatchFilesOptions::default()
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

		tokio::time::timeout(Duration::from_secs(5), watched.changed())
			.await
			.expect("watcher should report a symlink rotation")
			.then_some(())
			.expect("watcher channel should remain open");
	}
}
