use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use agent_core::prelude::*;
use agent_core::{drain, metrics, readiness, signal};
use prometheus_client::registry::Registry;
use tokio::task::JoinSet;

use crate::control::caclient;
use crate::telemetry::trc;
use crate::{Config, ProxyInputs, client, config_store, mcp, proxy, state_manager};

pub async fn run(
	config: Arc<Config>,
	config_resource_store: Option<config_store::ConfigResourceStore>,
) -> anyhow::Result<Bound> {
	crate::transport::tls::warn_if_key_log_enabled();
	let (data_plane_handle, data_plane_pool) = new_data_plane_pool(config.num_worker_threads);

	// Initialize OpenTelemetry resource defaults from gateway + proxy metadata
	trc::set_resource_defaults_from_config(config.as_ref());

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
		config.readiness_addr.clone(),
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
	agent_core::metrics::TokioCollector::register(sub_registry, &data_plane_handle);
	pprof_alloc::stats::cgroups::PrometheusCollector::register(sub_registry);
	pprof_alloc::stats::smaps::PrometheusCollector::register(sub_registry);

	// TODO: use for XDS
	let control_client = client::Client::new(&config.dns, None, config.backend.clone(), None);
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
	// Build metrics and then the upstream client with metrics wired in
	let sub_registry = metrics::sub_registry(&mut registry);
	let metrics_handle = Arc::new(crate::metrics::Metrics::new(
		sub_registry,
		config.metrics.excluded_metrics.clone(),
	));
	let client = client::Client::new(
		&config.dns,
		pool,
		config.backend.clone(),
		Some(metrics_handle.clone()),
	);

	let (xds_tx, xds_rx) = tokio::sync::watch::channel(());
	let state_mgr = state_manager::StateManager::new(
		config.clone(),
		control_client.clone(),
		Arc::new(xds_metrics),
		xds_tx,
		config_resource_store.clone(),
	)
	.await?;
	let stores = state_mgr.stores();
	let resource_manager = state_mgr.resource_manager();

	state_manager::start_self_workload_resolution(&config, stores.clone(), &ready);

	let mut model_catalog_sources = if let Some(store) = &config_resource_store {
		config_store::model_catalog_sources(
			&store
				.list(Some(config_store::ConfigResourceKind::ModelCatalog))
				.await?,
		)?
	} else {
		Vec::new()
	};
	model_catalog_sources.extend(config.model_catalog.sources.clone());
	let model_catalog = crate::llm::cost::ModelCatalog::new(model_catalog_sources)?;

	let mut xds_rx_for_task = xds_rx.clone();
	tokio::spawn(async move {
		// When we get the initial XDS state, unblock readiness
		let _ = xds_rx_for_task.changed().await;
		std::mem::drop(state_mgr_task);
	});
	// Run the XDS state manager in the current tokio worker pool.
	tokio::spawn(state_mgr.run());

	let admin_server = crate::management::admin::Service::new(
		config.clone(),
		model_catalog.clone(),
		config_resource_store,
		stores.clone(),
		resource_manager,
		shutdown.trigger(),
		drain_rx.clone(),
		data_plane_handle.clone(),
	)
	.await
	.context("admin server starts")?;
	#[cfg(feature = "ui")]
	info!("serving UI at {}", ui_url(config.as_ref()).await);

	let pi = ProxyInputs {
		cfg: config.clone(),
		stores: stores.clone(),
		metrics: metrics_handle.clone(),
		model_catalog,
		admin: Some(admin_server.service()),
		upstream: client.clone(),
		ca,

		mcp_state: mcp::App::new(stores.clone(), config.session_encoder.clone()),
	};

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
	let metrics_server = crate::management::metrics_server::Server::new(
		config.stats_addr.clone(),
		drain_rx.clone(),
		registry,
	)
	.await
	.context("stats server starts")?;
	// Run the metrics sever in the current tokio worker pool.
	metrics_server.spawn();
	Ok(Bound {
		drain_tx,
		shutdown,
		stores,
		ready,
	})
}

#[cfg(feature = "ui")]
async fn ui_url(config: &Config) -> String {
	let admin_url = || format!("http://{}/ui", config.admin_addr);
	let Some(local_config) = &config.xds.local_config else {
		return admin_url();
	};
	let Ok(contents) = local_config.read_to_string().await else {
		return admin_url();
	};
	let Ok(local) = crate::serdes::yamlviajson::from_str::<serde_json::Value>(&contents) else {
		return admin_url();
	};
	let gateway_ref = match local.pointer("/ui/gateways") {
		Some(serde_json::Value::String(reference)) => Some(reference.as_str()),
		Some(serde_json::Value::Array(references)) => {
			references.iter().find_map(serde_json::Value::as_str)
		},
		_ => None,
	}
	.or_else(|| {
		if local.get("ui").is_some() && local.pointer("/gateways/default").is_some() {
			Some("default")
		} else {
			None
		}
	});
	let Some(gateway_ref) = gateway_ref else {
		return admin_url();
	};
	let (gateway_name, listener_name) = gateway_ref
		.split_once('/')
		.map(|(gateway, listener)| (gateway, Some(listener)))
		.unwrap_or((gateway_ref, None));
	let Some(gateway) = local.get("gateways").and_then(|g| g.get(gateway_name)) else {
		return admin_url();
	};
	let Some(port) = gateway
		.get("port")
		.and_then(serde_json::Value::as_u64)
		.and_then(|port| u16::try_from(port).ok())
	else {
		return admin_url();
	};
	let endpoint = match listener_name {
		Some(listener_name) => gateway
			.get("listeners")
			.and_then(serde_json::Value::as_array)
			.and_then(|listeners| {
				listeners.iter().find(|listener| {
					listener.get("name").and_then(serde_json::Value::as_str) == Some(listener_name)
				})
			}),
		None => gateway
			.get("listeners")
			.and_then(serde_json::Value::as_array)
			.and_then(|listeners| listeners.first())
			.or(Some(gateway)),
	};
	let Some(endpoint) = endpoint else {
		return admin_url();
	};
	let hostname = endpoint
		.get("hostname")
		.and_then(serde_json::Value::as_str)
		.filter(|hostname| *hostname != "*")
		.unwrap_or("localhost");
	let scheme = if endpoint.get("tls").is_some_and(|tls| !tls.is_null()) {
		"https"
	} else {
		"http"
	};
	format!("{scheme}://{hostname}:{port}/ui")
}

pub struct Bound {
	pub shutdown: signal::Shutdown,
	drain_tx: drain::DrainTrigger,
	ready: readiness::Ready,
	stores: crate::store::Stores,
}

impl Bound {
	pub fn readiness(&self) -> readiness::Ready {
		self.ready.clone()
	}

	pub fn bind_addresses(&self) -> Vec<std::net::SocketAddr> {
		self.stores.binds.read().bind_addresses()
	}

	pub async fn wait_termination(self) -> anyhow::Result<()> {
		// Wait for a signal to shutdown from explicit admin shutdown or signal
		self.shutdown.wait().await;

		let sdp = {
			let b = self.stores.binds.read();
			b.all_shutdown_policies()
		};
		for p in sdp {
			p();
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

fn new_data_plane_pool(
	num_worker_threads: usize,
) -> (tokio::runtime::Handle, mpsc::Sender<DataPlaneTask>) {
	let (tx, rx) = mpsc::channel();
	let (tx_handle, rx_handle) = mpsc::channel();

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
		tx_handle.send(runtime.handle().clone()).unwrap();
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

	let handle = rx_handle.recv().unwrap();
	(handle, tx)
}
