use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use agent_core::prelude::*;
use agent_core::{drain, metrics, readiness, signal, trcng};
use prometheus_client::registry::Registry;
use tokio::task::JoinSet;

use crate::control::caclient;
use crate::telemetry::trc;
use crate::telemetry::trc::Tracer;
use crate::{Config, ProxyInputs, client, mcp, proxy, state_manager};

pub async fn run(config: Arc<Config>) -> anyhow::Result<Bound> {
	let data_plane_pool = new_data_plane_pool(config.num_worker_threads);

	// TODO consolidate this
	trcng::init_tracer(trcng::Config {
		tracer: trcng::Tracer::Otlp {
			endpoint: config.tracing.endpoint.clone(),
		},
		tags: Default::default(),
	})?;
	let shutdown = signal::Shutdown::new();
	// Setup a drain channel. drain_tx is used to trigger a drain, which will complete
	// once all drain_rx handlers are dropped.
	// Any component which wants time to gracefully exit should take in a drain_rx clone,
	// await drain_rx.signaled(), then cleanup.
	// Note: there is still a hard timeout if the draining takes too long
	let (drain_tx, drain_rx) = drain::new();

	let ready = readiness::Ready::new();
	let state_mgr_task = ready.register_task("state manager");
	let proxy_task = ready.register_task("agentgateway");

	let readiness_server = crate::management::readiness_server::Server::new(
		config.readiness_addr,
		drain_rx.clone(),
		ready.clone(),
	)
	.await
	.context("readiness server starts")?;
	// Run the readiness server in the data plane worker pool.
	data_plane_pool.send(DataPlaneTask {
		block_shutdown: false,
		fut: Box::pin(async move {
			readiness_server.spawn();
			Ok(())
		}),
	})?;

	let mut registry = Registry::default();
	let sub_registry = metrics::sub_registry(&mut registry);
	let xds_metrics = agent_xds::Metrics::new(sub_registry);
	// TODO: metric for version

	// TODO: use for XDS
	let control_client = client::Client::new(&config.dns, None);
	let ca = if let Some(cfg) = &config.ca {
		Some(Arc::new(caclient::CaClient::new(
			control_client.clone(),
			cfg.clone(),
		)?))
	} else {
		None
	};
	let pool = ca
		.clone()
		.map(|ca| agent_hbone::pool::WorkloadHBONEPool::new(config.hbone.clone(), ca));
	let client = client::Client::new(&config.dns, pool);

	let (xds_tx, xds_rx) = tokio::sync::watch::channel(());
	let state_mgr =
		state_manager::StateManager::new(&config.xds, client.clone(), xds_metrics, xds_tx).await?;
	let mut xds_rx_for_task = xds_rx.clone();
	tokio::spawn(async move {
		// When we get the initial XDS state, unblock readiness
		let _ = xds_rx_for_task.changed().await;
		std::mem::drop(state_mgr_task);
	});
	let stores = state_mgr.stores();
	// Run the XDS state manager in the current tokio worker pool.
	tokio::spawn(state_mgr.run());

	#[allow(unused_mut)]
	let mut admin_server = crate::management::admin::Service::new(
		config.clone(),
		stores.clone(),
		shutdown.trigger(),
		drain_rx.clone(),
	)
	.await
	.context("admin server starts")?;
	// Initialize admin JWT (if configured via nested config or env). Previously the field existed but
	// we never converted LocalJwtConfig -> Jwt or installed it, so claims were not injected (has_claims=false).
	if let Some(local_cfg) = config.admin_jwt.clone() {
		match local_cfg.clone().try_into(client.clone()).await {
			Ok(jwt) => {
				admin_server.set_admin_jwt(jwt);
				info!(issuer=%local_cfg.issuer, audiences=?local_cfg.audiences, "admin jwt configured");
			},
			Err(e) => {
				warn!(error=?e, "failed to initialize admin jwt (admin surface will have no JWT claims)");
			},
		}
	}
	#[cfg(feature = "ui")]
	admin_server.set_admin_handler(Arc::new(crate::ui::UiHandler::new(config.clone())));
	#[cfg(feature = "ui")]
	info!("serving UI at http://{}/ui", config.admin_addr);

	let sub_registry = metrics::sub_registry(&mut registry);
	let tracer = trc::Tracer::new(&config.tracing)?;
	#[allow(unused_mut)]
	let mut pi = ProxyInputs {
		cfg: config.clone(),
		stores: stores.clone(),
		tracer: tracer.clone(),
		metrics: Arc::new(crate::metrics::Metrics::new(sub_registry)),
		upstream: client.clone(),
		ca,
		#[cfg(feature = "pat")]
		db_pool: None, // initialized later if enabled

		mcp_state: mcp::sse::App::new(
			stores.clone(),
			Arc::new(crate::mcp::relay::metrics::Metrics::new(
				&mut registry,
				None, // TODO custom tags
			)),
			drain_rx.clone(),
		),
	};

	#[cfg(feature = "pat")]
	let mut pat_pool_for_admin: Option<Arc<sqlx::PgPool>> = None;
	#[cfg(feature = "pat")]
	if config.pat.enabled {
		if let Some(db) = &config.database {
			if let Some(url) = &db.url {
				// Provide dedicated URL to PAT revocation listener before pool usage
				crate::http::pat::set_pat_db_url(url);
				// Retry loop to tolerate transient DB unavailability at startup
				let max_attempts: u32 = std::env::var("PAT_DB_INIT_MAX_ATTEMPTS")
					.ok()
					.and_then(|v| v.parse().ok())
					.unwrap_or(5);
				let mut attempt = 0u32;
				let mut last_err: Option<anyhow::Error> = None;
				while attempt < max_attempts {
					attempt += 1;
					match sqlx::postgres::PgPoolOptions::new()
						.max_connections(db.max_connections)
						.connect(url)
						.await
					{
						Ok(pool) => {
							let arc_pool = Arc::new(pool);
							pi.db_pool = Some(arc_pool.clone());
							pat_pool_for_admin = Some(arc_pool.clone());
							// Initialize global PAT auth singleton for cache sharing
							crate::pat_global::init_global_pat_auth((*arc_pool).clone());
							info!(attempt, "PAT db pool and global authenticator initialized");
							last_err = None;
							break;
						},
						Err(e) => {
							warn!(attempt, error=?e, "failed to init PAT db pool (will retry)");
							last_err = Some(e.into());
							let backoff_ms = 200u64.saturating_mul(2u64.pow(attempt.min(6))); // up to ~12.8s
							if attempt < max_attempts {
								tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
							}
						},
					}
				}
				if last_err.is_some() {
					warn!("giving up initializing PAT db pool");
				}
			}
		}
	}
	#[cfg(feature = "pat")]
	if let Some(p) = pat_pool_for_admin.clone() {
		admin_server.set_pat_db(p);
	}

	let gw = proxy::Gateway::new(Arc::new(pi), drain_rx.clone());

	// Run the agentgateway in the data plane worker pool.
	let mut xds_rx_for_proxy = xds_rx.clone();
	data_plane_pool.send(DataPlaneTask {
		block_shutdown: true,
		fut: Box::pin(async move {
			// Wait for XDS to be ready
			let _ = xds_rx_for_proxy.changed().await;
			// Now run
			gw.run().in_current_span().await;
			Ok(())
		}),
	})?;

	drop(proxy_task);

	// Run the admin server in the current tokio worker pool.
	admin_server.spawn();

	// Create and start the metrics server.
	let metrics_server =
		crate::management::metrics_server::Server::new(config.stats_addr, drain_rx.clone(), registry)
			.await
			.context("stats server starts")?;
	// Run the metrics sever in the current tokio worker pool.
	metrics_server.spawn();
	tokio::task::spawn_blocking(|| {
		let t0 = std::time::Instant::now();
		crate::llm::preload_tokenizers();
		debug!("tokenizers loaded in {}ms", t0.elapsed().as_millis());
	});
	Ok(Bound {
		drain_tx,
		shutdown,
		tracer,
	})
}

pub struct Bound {
	pub shutdown: signal::Shutdown,
	drain_tx: drain::DrainTrigger,
	tracer: Option<Tracer>,
}

impl Bound {
	pub async fn wait_termination(self) -> anyhow::Result<()> {
		// Wait for a signal to shutdown from explicit admin shutdown or signal
		self.shutdown.wait().await;

		if let Some(tracer) = self.tracer {
			tracer.shutdown()
		}

		// Start a drain; this will attempt to end all connections
		// or itself be interrupted by a stronger TERM signal, whichever comes first.
		self
			.drain_tx
			.start_drain_and_wait(drain::DrainMode::Graceful)
			.await;

		Ok(())
	}
}

struct DataPlaneTask {
	block_shutdown: bool,
	fut: Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + Sync + 'static>>,
}

fn new_data_plane_pool(num_worker_threads: usize) -> mpsc::Sender<DataPlaneTask> {
	let (tx, rx) = mpsc::channel();

	let span = tracing::span::Span::current();
	thread::spawn(move || {
		let _span = span.enter();
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(num_worker_threads)
			.thread_name_fn(|| {
				static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
				let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
				format!("agentgateway-{id}")
			})
			.enable_all()
			.build()
			.unwrap();
		runtime.block_on(
			async move {
				let mut join_set = JoinSet::new();

				// Spawn tasks as they're received, until all tasks are spawned.
				let task_iter: mpsc::Iter<DataPlaneTask> = rx.iter();
				for task in task_iter {
					if task.block_shutdown {
						// We'll block shutdown on this task.
						join_set.spawn(task.fut);
					} else {
						// We won't block shutdown of this task. Just spawn and forget.
						tokio::spawn(task.fut);
					}
				}

				while let Some(join_result) = join_set.join_next().await {
					match join_result {
						Ok(result) => {
							if let Err(e) = result {
								warn!("data plane task failed: {e}");
							}
						},
						Err(e) => warn!("failed joining data plane task: {e}"),
					}
				}
			}
			.in_current_span(),
		);
	});

	tx
}
