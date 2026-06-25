use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::future::pending;
use std::path::{Path, PathBuf, absolute};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, anyhow};
use bytes::Bytes;
use headers::{CacheControl, HeaderMapExt};
use http::header::EXPIRES;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};
use tracing::{trace, warn};

use crate::client::Client;
use crate::http::Body;

const JWKS_TTL: Duration = Duration::from_mins(15);
const OPENAPI_TTL: Duration = Duration::from_hours(24);
const GENERIC_TTL: Duration = Duration::from_mins(15);
const FAILED_HTTP_REFRESH: Duration = Duration::from_secs(15);
const MIN_HTTP_REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ResourceKind {
	Generic,
	Jwks,
	OpenApi,
	OidcDiscovery,
}

impl ResourceKind {
	fn default_ttl(self) -> Duration {
		match self {
			ResourceKind::OpenApi => OPENAPI_TTL,
			ResourceKind::Jwks => JWKS_TTL,
			ResourceKind::Generic | ResourceKind::OidcDiscovery => GENERIC_TTL,
		}
	}
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum ResourceRef {
	File(PathBuf),
	Http { url: http::Uri, kind: ResourceKind },
}

#[derive(Clone, Debug, Default)]
pub struct ResourceChange {
	pub version: u64,
	pub resource: String,
}

#[derive(Clone)]
pub struct ResourceManager {
	inner: Arc<Inner>,
}

impl std::fmt::Debug for ResourceManager {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("ResourceManager").finish_non_exhaustive()
	}
}

#[derive(Clone, Debug)]
pub struct ResourceFetcher {
	mode: ResourceFetcherMode,
	// Populated only while a ResourceFetchScope is active for managed fetchers.
	// It records the resources the successful config should retain.
	tracking: Arc<std::sync::Mutex<Option<HashSet<ResourceRef>>>>,
}

pub struct ResourceFetchScope<'a> {
	fetcher: &'a ResourceFetcher,
	active: bool,
}

#[derive(Clone, Debug)]
enum ResourceFetcherMode {
	Managed(ResourceManager),
	CachedOrDirect(ResourceManager),
	Direct(Box<Client>),
	FilesOnly,
}

impl ResourceFetcher {
	/// Uses the manager cache and starts refresh/watch behavior for runtime config dependencies.
	pub fn managed(manager: ResourceManager) -> Self {
		Self {
			mode: ResourceFetcherMode::Managed(manager),
			tracking: Default::default(),
		}
	}

	/// Reads from the manager cache when present, otherwise fetches once without retaining or watching.
	pub fn cached_or_direct(manager: ResourceManager) -> Self {
		Self {
			mode: ResourceFetcherMode::CachedOrDirect(manager),
			tracking: Default::default(),
		}
	}

	/// Fetches resources directly with the client and does not use the manager cache.
	pub fn direct(client: Client) -> Self {
		Self {
			mode: ResourceFetcherMode::Direct(Box::new(client)),
			tracking: Default::default(),
		}
	}

	/// Reads local files directly and rejects HTTP resources.
	pub fn files_only() -> Self {
		Self {
			mode: ResourceFetcherMode::FilesOnly,
			tracking: Default::default(),
		}
	}

	/// Loads a normalized resource according to this fetcher's mode.
	pub async fn fetch(&self, resource: ResourceRef) -> anyhow::Result<Bytes> {
		let normalized = normalize_resource(resource)?;
		match &self.mode {
			ResourceFetcherMode::Managed(manager) => {
				self.track_resource(normalized.clone());
				manager.fetch_and_wait_normalized(normalized).await
			},
			ResourceFetcherMode::CachedOrDirect(manager) => {
				manager.fetch_cached_or_direct_normalized(normalized).await
			},
			ResourceFetcherMode::Direct(client) => fetch_direct(client.as_ref(), &normalized)
				.await
				.map(|r| r.content),
			ResourceFetcherMode::FilesOnly => match normalized {
				ResourceRef::File(path) => fs_err::tokio::read(&path)
					.await
					.with_context(|| format!("read resource file {}", path.display()))
					.map(Bytes::from),
				ResourceRef::Http { url, .. } => {
					Err(anyhow!("resource fetcher cannot fetch HTTP resource {url}"))
				},
			},
		}
	}

	/// Records managed resource lookups during a full config computation.
	/// The returned guard commits fetched resources on success, or restores the
	/// last committed set on failure so failed reloads do not leave stale state.
	pub fn scope_full_computation(&self) -> ResourceFetchScope<'_> {
		if !matches!(self.mode, ResourceFetcherMode::Managed(_)) {
			return ResourceFetchScope {
				fetcher: self,
				active: false,
			};
		}

		let mut tracking = self
			.tracking
			.lock()
			.expect("resource fetcher tracking mutex poisoned");
		*tracking = Some(HashSet::new());
		ResourceFetchScope {
			fetcher: self,
			active: true,
		}
	}

	fn take_tracked_resources(&self) -> Option<HashSet<ResourceRef>> {
		let mut tracking = self
			.tracking
			.lock()
			.expect("resource fetcher tracking mutex poisoned");
		tracking.take()
	}

	fn track_resource(&self, resource: ResourceRef) {
		if let Some(tracking) = self
			.tracking
			.lock()
			.expect("resource fetcher tracking mutex poisoned")
			.as_mut()
		{
			tracking.insert(resource);
		}
	}
}

impl ResourceFetchScope<'_> {
	pub fn finish(mut self, success: bool) {
		if !self.active {
			return;
		}
		self.active = false;
		let fetched = self.fetcher.take_tracked_resources();
		if let (Some(fetched), ResourceFetcherMode::Managed(manager)) = (fetched, &self.fetcher.mode) {
			if success {
				manager.retain_resources(fetched);
			} else {
				manager.retain_active_resources();
			}
		}
	}
}

impl Drop for ResourceFetchScope<'_> {
	fn drop(&mut self) {
		if self.active {
			self.fetcher.take_tracked_resources();
			if let ResourceFetcherMode::Managed(manager) = &self.fetcher.mode {
				manager.retain_active_resources();
			}
		}
	}
}

struct Inner {
	client: Client,
	entries: Mutex<HashMap<ResourceRef, Entry>>,
	// Resources referenced by the last successfully normalized local config.
	// Background refreshes check this before publishing changes.
	active_resources: Mutex<HashSet<ResourceRef>>,
	watched_files: FileWatchRegistry,
	scheduler_tx: mpsc::UnboundedSender<ScheduledRefresh>,
	change_tx: watch::Sender<ResourceChange>,
	change_counter: AtomicU64,
}

struct Entry {
	content: Bytes,
	next_refresh: Option<Instant>,
}

struct FileWatch {
	id: u64,
	task: JoinHandle<()>,
}

#[derive(Default)]
struct FileWatchRegistry {
	watches: std::sync::Mutex<HashMap<PathBuf, FileWatch>>,
	counter: AtomicU64,
}

impl FileWatchRegistry {
	// Watch tasks can outlive their map entry. IDs keep old tasks from removing
	// a newer watch registered for the same path after Vim-style file replacement.
	fn next_id(&self) -> u64 {
		self.counter.fetch_add(1, AtomicOrdering::Relaxed) + 1
	}

	fn contains(&self, path: &Path) -> bool {
		self
			.watches
			.lock()
			.expect("resource file watcher mutex poisoned")
			.contains_key(path)
	}

	fn insert_or_abort(&self, path: PathBuf, watch: FileWatch) {
		let mut watches = self
			.watches
			.lock()
			.expect("resource file watcher mutex poisoned");
		if watches.contains_key(&path) {
			watch.task.abort();
			return;
		}
		watches.insert(path, watch);
	}

	fn remove_if_current(&self, path: &Path, id: u64) {
		let mut watches = self
			.watches
			.lock()
			.expect("resource file watcher mutex poisoned");
		if watches.get(path).is_some_and(|watch| watch.id == id) {
			watches.remove(path);
		}
	}

	fn retain_paths(&self, retained: &HashSet<PathBuf>) {
		let mut watches = self
			.watches
			.lock()
			.expect("resource file watcher mutex poisoned");
		watches.retain(|path, watch| {
			let retain = retained.contains(path);
			if !retain {
				watch.task.abort();
			}
			retain
		});
	}

	#[cfg(test)]
	fn len(&self) -> usize {
		self
			.watches
			.lock()
			.expect("resource file watcher mutex poisoned")
			.len()
	}
}

#[derive(Debug)]
struct ScheduledRefresh {
	at: Instant,
	resource: ResourceRef,
}

#[derive(Debug, PartialEq, Eq)]
struct TimerEntry {
	at: Instant,
	resource: ResourceRef,
}

impl PartialOrd for TimerEntry {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}
impl Ord for TimerEntry {
	fn cmp(&self, other: &Self) -> Ordering {
		other
			.at
			.cmp(&self.at)
			.then_with(|| resource_key(&self.resource).cmp(&resource_key(&other.resource)))
	}
}

impl ResourceManager {
	pub fn new(client: Client) -> anyhow::Result<Self> {
		let (scheduler_tx, scheduler_rx) = mpsc::unbounded_channel();
		let (change_tx, _) = watch::channel(ResourceChange::default());
		let manager = Self {
			inner: Arc::new(Inner {
				client,
				entries: Default::default(),
				active_resources: Default::default(),
				watched_files: Default::default(),
				scheduler_tx,
				change_tx,
				change_counter: AtomicU64::new(0),
			}),
		};
		manager.start_http_scheduler(scheduler_rx);
		Ok(manager)
	}

	pub fn subscribe_changes(&self) -> watch::Receiver<ResourceChange> {
		self.inner.change_tx.subscribe()
	}

	pub async fn fetch_and_wait(&self, resource: ResourceRef) -> anyhow::Result<Bytes> {
		let resource = normalize_resource(resource)?;
		self.fetch_and_wait_normalized(resource).await
	}

	async fn fetch_and_wait_normalized(&self, resource: ResourceRef) -> anyhow::Result<Bytes> {
		if let ResourceRef::File(path) = &resource {
			self.watch_file(path)?;
		}
		if let Some(content) = self.cached(&resource) {
			return Ok(content);
		}
		let FetchResult { content, next } = self.fetch(&resource).await?;
		self.store(resource, content.clone(), next);
		Ok(content)
	}

	pub async fn fetch_cached_or_direct(&self, resource: ResourceRef) -> anyhow::Result<Bytes> {
		let resource = normalize_resource(resource)?;
		self.fetch_cached_or_direct_normalized(resource).await
	}

	async fn fetch_cached_or_direct_normalized(
		&self,
		resource: ResourceRef,
	) -> anyhow::Result<Bytes> {
		if let Some(content) = self.cached(&resource) {
			return Ok(content);
		}
		fetch_direct(&self.inner.client, &resource)
			.await
			.map(|r| r.content)
	}

	pub fn retain_resources(&self, retained: HashSet<ResourceRef>) {
		*self
			.inner
			.active_resources
			.lock()
			.expect("resource active set mutex poisoned") = retained.clone();
		self.retain_cached_and_watched_resources(&retained);
	}

	fn retain_active_resources(&self) {
		// Failed reloads may have fetched temporary resources before erroring.
		// Prune back to the last successful dependency set.
		let retained = self
			.inner
			.active_resources
			.lock()
			.expect("resource active set mutex poisoned")
			.clone();
		self.retain_cached_and_watched_resources(&retained);
	}

	fn retain_cached_and_watched_resources(&self, retained: &HashSet<ResourceRef>) {
		self
			.inner
			.entries
			.lock()
			.expect("resource cache mutex poisoned")
			.retain(|resource, _| retained.contains(resource));

		let retained_files = retained
			.iter()
			.filter_map(|resource| match resource {
				ResourceRef::File(path) => Some(path.clone()),
				ResourceRef::Http { .. } => None,
			})
			.collect::<HashSet<_>>();
		self.inner.watched_files.retain_paths(&retained_files);
	}

	fn cached(&self, resource: &ResourceRef) -> Option<Bytes> {
		self
			.inner
			.entries
			.lock()
			.expect("resource cache mutex poisoned")
			.get(resource)
			.map(|e| e.content.clone())
	}

	fn store(&self, resource: ResourceRef, content: Bytes, next: Option<Instant>) {
		self
			.inner
			.entries
			.lock()
			.expect("resource cache mutex poisoned")
			.insert(
				resource.clone(),
				Entry {
					content,
					next_refresh: next,
				},
			);
		if let Some(at) = next {
			tracing::debug!(
				resource = %resource_key(&resource),
				refresh_in = ?at.saturating_duration_since(Instant::now()),
				"scheduled resource refresh"
			);
			let _ = self
				.inner
				.scheduler_tx
				.send(ScheduledRefresh { at, resource });
		}
	}

	async fn refetch_and_notify_if_changed(&self, resource: ResourceRef) {
		// A resource can be forgotten while an HTTP refresh is queued or in flight.
		// Check before and after I/O so stale refreshes cannot resurrect it.
		if !self.is_active(&resource) {
			return;
		}
		let result = self.fetch(&resource).await;
		let FetchResult { content, next } = match result {
			Ok(result) => result,
			Err(e) => {
				warn!(resource = %resource_key(&resource), "failed to refresh resource: {e}");
				if !self.is_active(&resource) {
					return;
				}
				let next = Instant::now() + FAILED_HTTP_REFRESH;
				let _ = self
					.inner
					.scheduler_tx
					.send(ScheduledRefresh { at: next, resource });
				return;
			},
		};
		if !self.is_active(&resource) {
			return;
		}

		let changed = {
			let mut entries = self
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned");
			let changed = entries
				.get(&resource)
				.is_none_or(|entry| entry.content != content);
			entries.insert(
				resource.clone(),
				Entry {
					content,
					next_refresh: next,
				},
			);
			changed
		};
		if let Some(at) = next {
			tracing::debug!(
				resource = %resource_key(&resource),
				refresh_in = ?at.saturating_duration_since(Instant::now()),
				"scheduled resource refresh"
			);
			let _ = self.inner.scheduler_tx.send(ScheduledRefresh {
				at,
				resource: resource.clone(),
			});
		}
		if changed {
			self.notify_changed(&resource);
		}
	}

	async fn fetch(&self, resource: &ResourceRef) -> anyhow::Result<FetchResult> {
		fetch_direct(&self.inner.client, resource).await
	}

	fn is_active(&self, resource: &ResourceRef) -> bool {
		self
			.inner
			.active_resources
			.lock()
			.expect("resource active set mutex poisoned")
			.contains(resource)
	}

	fn watch_file(&self, path: &Path) -> anyhow::Result<()> {
		let abspath = absolute(path)?;
		trace!("adding file watch for {}", path.display());
		let watch_id = self.inner.watched_files.next_id();
		if self.inner.watched_files.contains(&abspath) {
			return Ok(());
		}
		let mut watched = crate::util::watch_files_with_options(
			vec![abspath.clone()],
			crate::util::WatchFilesOptions::default()
				// Managed dependencies should reload when a referenced file disappears.
				.reload_on_disappearance(true)
				// Vim-style replace writes invalidate the inode watch; close it so
				// the resource manager can register a fresh watch for the new file.
				.close_on_removal(true),
		)?;

		let manager = self.clone();
		let path = abspath.clone();
		let task = tokio::spawn(async move {
			while let Some(invalidated) = watched.changed_invalidated().await {
				if invalidated {
					// Linux file watches are tied to the replaced inode. Remove this
					// task's entry and register a fresh watch for the new file.
					manager
						.inner
						.watched_files
						.remove_if_current(&path, watch_id);
					if let Err(e) = manager.watch_file(&path) {
						warn!("failed to re-watch resource file {}: {e}", path.display());
					}
				}
				manager.refresh_file(path.clone()).await;
				if invalidated {
					break;
				}
			}
			manager
				.inner
				.watched_files
				.remove_if_current(&path, watch_id);
		});
		self
			.inner
			.watched_files
			.insert_or_abort(abspath, FileWatch { id: watch_id, task });

		Ok(())
	}

	async fn refresh_file(&self, path: PathBuf) {
		let resource = ResourceRef::File(path.clone());
		// The file watch event may race with dependency cleanup.
		if !self.is_active(&resource) {
			return;
		}
		let content = match fs_err::tokio::read(&path).await {
			Ok(content) => Bytes::from(content),
			Err(e) => {
				warn!("failed to refresh resource file {}: {e}", path.display());
				if !self.is_active(&resource) {
					return;
				}
				let removed = self
					.inner
					.entries
					.lock()
					.expect("resource cache mutex poisoned")
					.remove(&resource)
					.is_some();
				if removed {
					self.notify_changed(&resource);
				}
				return;
			},
		};
		if !self.is_active(&resource) {
			return;
		}
		let changed = {
			let mut entries = self
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned");
			let changed = entries
				.get(&resource)
				.is_none_or(|entry| entry.content != content);
			entries.insert(
				resource.clone(),
				Entry {
					content,
					next_refresh: None,
				},
			);
			changed
		};
		if changed {
			self.notify_changed(&resource);
		}
	}

	fn start_http_scheduler(&self, mut scheduler_rx: mpsc::UnboundedReceiver<ScheduledRefresh>) {
		let manager = self.clone();
		tokio::spawn(async move {
			let mut heap = BinaryHeap::<TimerEntry>::new();
			loop {
				let next = heap.peek().map(|entry| entry.at);
				tokio::select! {
					_ = maybe_sleep_until(next) => {
						let Some(entry) = heap.pop() else {
							continue;
						};
						if manager.should_refresh(&entry.resource, entry.at) {
							manager.refetch_and_notify_if_changed(entry.resource).await;
						}
					}
					scheduled = scheduler_rx.recv() => {
						let Some(scheduled) = scheduled else {
							return;
						};
						heap.push(TimerEntry {
							at: scheduled.at,
							resource: scheduled.resource,
						});
					}
				}
			}
		});
	}

	fn should_refresh(&self, resource: &ResourceRef, at: Instant) -> bool {
		if !self.is_active(resource) {
			return false;
		}
		self
			.inner
			.entries
			.lock()
			.expect("resource cache mutex poisoned")
			.get(resource)
			.and_then(|entry| entry.next_refresh)
			.is_some_and(|current| current == at)
	}

	fn notify_changed(&self, resource: &ResourceRef) {
		let next = self
			.inner
			.change_counter
			.fetch_add(1, AtomicOrdering::Relaxed)
			+ 1;
		let _ = self.inner.change_tx.send(ResourceChange {
			version: next,
			resource: resource_key(resource),
		});
	}
}

struct FetchResult {
	content: Bytes,
	next: Option<Instant>,
}

async fn fetch_direct(client: &Client, resource: &ResourceRef) -> anyhow::Result<FetchResult> {
	match resource {
		ResourceRef::File(path) => {
			let content = fs_err::tokio::read(path)
				.await
				.with_context(|| format!("read resource file {}", path.display()))?;
			Ok(FetchResult {
				content: content.into(),
				next: None,
			})
		},
		ResourceRef::Http { url, kind } => {
			let resp = client
				.simple_call(
					::http::Request::builder()
						.uri(url)
						.body(Body::empty())
						.expect("builder should succeed"),
				)
				.await
				.with_context(|| format!("fetch {url}"))?;
			let ttl = ttl_from_headers(resp.headers(), kind.default_ttl());
			let body = crate::http::read_resp_body(resp).await?;
			Ok(FetchResult {
				content: body,
				next: Some(Instant::now() + ttl.max(MIN_HTTP_REFRESH)),
			})
		},
	}
}

fn ttl_from_headers(headers: &http::HeaderMap, default: Duration) -> Duration {
	if let Some(ttl) = cache_control_max_age(headers) {
		return ttl;
	}
	if let Some(expires) = headers.get(EXPIRES).and_then(|value| value.to_str().ok())
		&& let Ok(expires) = httpdate::parse_http_date(expires)
		&& let Ok(ttl) = expires.duration_since(std::time::SystemTime::now())
	{
		return ttl;
	}
	default
}

fn cache_control_max_age(headers: &http::HeaderMap) -> Option<Duration> {
	let cache_control = headers.typed_get::<CacheControl>()?;
	if cache_control.no_cache() || cache_control.no_store() {
		return None;
	}
	cache_control.max_age()
}

fn resource_key(resource: &ResourceRef) -> String {
	match resource {
		ResourceRef::File(path) => format!("file:{}", path.display()),
		ResourceRef::Http { url, kind } => format!("http:{kind:?}:{url}"),
	}
}

fn normalize_resource(resource: ResourceRef) -> anyhow::Result<ResourceRef> {
	Ok(match resource {
		ResourceRef::File(path) => ResourceRef::File(absolute(path)?),
		ResourceRef::Http { url, kind } => ResourceRef::Http { url, kind },
	})
}

async fn maybe_sleep_until(till: Option<Instant>) {
	if let Some(till) = till {
		sleep_until(till).await;
	} else {
		pending::<()>().await;
	}
}

#[cfg(test)]
mod tests {
	use http::header::CACHE_CONTROL;

	use super::*;

	fn test_client() -> Client {
		Client::new(
			&crate::client::Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
			},
			None,
			crate::BackendConfig::default(),
			None,
		)
	}

	async fn scoped<T>(
		resources: &ResourceFetcher,
		f: impl AsyncFnOnce() -> anyhow::Result<T>,
	) -> anyhow::Result<T> {
		let scope = resources.scope_full_computation();
		let result = f().await;
		scope.finish(result.is_ok());
		result
	}

	#[test]
	fn default_ttls_are_per_resource_kind() {
		assert_eq!(
			ResourceKind::Jwks.default_ttl(),
			Duration::from_secs(15 * 60)
		);
		assert_eq!(
			ResourceKind::OpenApi.default_ttl(),
			Duration::from_secs(24 * 60 * 60)
		);
	}

	#[test]
	fn cache_control_max_age_overrides_default_ttl() {
		let mut headers = http::HeaderMap::new();
		headers.insert(CACHE_CONTROL, "public, max-age=42".parse().unwrap());

		assert_eq!(
			ttl_from_headers(&headers, Duration::from_secs(5)),
			Duration::from_secs(42)
		);
	}

	#[test]
	fn cache_control_no_cache_falls_back_to_default_ttl() {
		let mut headers = http::HeaderMap::new();
		headers.insert(CACHE_CONTROL, "no-cache".parse().unwrap());

		assert_eq!(
			ttl_from_headers(&headers, Duration::from_secs(5)),
			Duration::from_secs(5)
		);
	}

	#[tokio::test]
	async fn tracked_normalization_forgets_only_after_successful_commit() {
		let dir = tempfile::tempdir().unwrap();
		let a = dir.path().join("a.pem");
		let b = dir.path().join("b.pem");
		fs_err::write(&a, "a").unwrap();
		fs_err::write(&b, "b").unwrap();

		let manager = ResourceManager::new(test_client()).unwrap();
		let resources = ResourceFetcher::managed(manager.clone());
		let a_resource = normalize_resource(ResourceRef::File(a.clone())).unwrap();
		let b_resource = normalize_resource(ResourceRef::File(b.clone())).unwrap();

		scoped(&resources, || async {
			resources.fetch(ResourceRef::File(a.clone())).await?;
			resources.fetch(ResourceRef::File(b.clone())).await?;
			Ok(())
		})
		.await
		.unwrap();
		assert!(
			manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&a_resource)
		);
		assert!(
			manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&b_resource)
		);
		assert_eq!(manager.inner.watched_files.len(), 2);

		scoped(&resources, || async {
			resources.fetch(ResourceRef::File(a.clone())).await?;
			Ok(())
		})
		.await
		.unwrap();
		assert!(
			manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&a_resource)
		);
		assert!(
			!manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&b_resource)
		);
		assert_eq!(manager.inner.watched_files.len(), 1);

		let failed: anyhow::Result<()> = scoped(&resources, || async {
			resources.fetch(ResourceRef::File(b.clone())).await?;
			anyhow::bail!("conversion failed")
		})
		.await;
		assert!(failed.is_err());
		assert!(
			manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&a_resource)
		);
		assert!(
			!manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&b_resource)
		);
		assert_eq!(manager.inner.watched_files.len(), 1);
	}

	#[tokio::test]
	async fn file_refresh_failure_evicts_cached_content_and_notifies() {
		let dir = tempfile::tempdir().unwrap();
		let file = dir.path().join("cert.pem");
		fs_err::write(&file, "cert").unwrap();

		let manager = ResourceManager::new(test_client()).unwrap();
		let resources = ResourceFetcher::managed(manager.clone());
		let resource = normalize_resource(ResourceRef::File(file.clone())).unwrap();
		scoped(&resources, || async {
			resources.fetch(ResourceRef::File(file.clone())).await?;
			Ok(())
		})
		.await
		.unwrap();
		assert!(
			manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&resource)
		);

		let mut changes = manager.subscribe_changes();
		fs_err::remove_file(&file).unwrap();
		manager.refresh_file(file).await;

		assert!(
			!manager
				.inner
				.entries
				.lock()
				.expect("resource cache mutex poisoned")
				.contains_key(&resource)
		);
		tokio::time::timeout(Duration::from_secs(1), changes.changed())
			.await
			.expect("resource deletion should notify")
			.expect("change channel should remain open");
	}
}
