use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, env};

use agent_core::durfmt;
use agent_core::env::ENV;
use agent_core::prelude::*;
use secrecy::ExposeSecret;

use crate::control::caclient;
use crate::telemetry::log::{LoggingFields, MetricFields};
use crate::telemetry::trc;
use crate::types;
use crate::types::discovery::{Identity, WaypointIdentity};
use crate::{
	Address, Config, ConfigSource, DnsLookupFamily, NestedRawConfig, RawLoggingLevel, StringOrInt,
	ThreadingMode, XDSConfig, cel, client, serdes, telemetry,
};

#[derive(Default)]
struct TracingEnvOverrides {
	endpoint: Option<String>,
	headers: Option<std::collections::HashMap<String, String>>,
	protocol: Option<trc::Protocol>,
}

pub fn parse_config(contents: String, filename: Option<PathBuf>) -> anyhow::Result<Config> {
	let nested: NestedRawConfig = serdes::yamlviajson::from_str(&contents)?;
	let raw = nested.config.unwrap_or_default();

	let ipv6_enabled = parse::<bool>("IPV6_ENABLED")?
		.or(raw.enable_ipv6)
		.unwrap_or(true);
	let ipv6_localhost_enabled = if ipv6_enabled {
		// IPv6 may be generally enabled, but not on localhost. In that case, we do not want to bind on IPv6.
		crate::ipv6_enabled_on_localhost().unwrap_or_else(|e| {
			warn!(err=?e, "failed to determine if IPv6 was disabled; continuing anyways, but this may fail");
			true
		})
	} else {
		false
	};
	let bind_wildcard = if ipv6_enabled {
		IpAddr::V6(Ipv6Addr::UNSPECIFIED)
	} else {
		IpAddr::V4(Ipv4Addr::UNSPECIFIED)
	};
	let local_config = parse::<PathBuf>("LOCAL_XDS_PATH")?
		.or(raw.local_xds_path)
		.or(filename)
		.map(ConfigSource::File);

	let dns = raw.dns.unwrap_or_default();
	let dns_lookup_family = match env::var("DNS_LOOKUP_FAMILY") {
		Ok(val) => Some(DnsLookupFamily::from_env_str(&val)?),
		Err(_) => None,
	}
	.or(dns.lookup_family)
	.unwrap_or_default();
	let dns_edns0: Option<bool> = parse("DNS_EDNS0")?.or(dns.edns0);
	let (resolver_cfg, resolver_opts) = {
		let (cfg, opts) = hickory_resolver::system_conf::read_system_conf().unwrap_or_else(|e| {
			warn!(err=?e, "failed to read system DNS config, using defaults");
			(
				hickory_resolver::config::ResolverConfig::default(),
				hickory_resolver::config::ResolverOpts::default(),
			)
		});
		resolve_dns_config(cfg, opts, dns_lookup_family, ipv6_enabled, dns_edns0)
	};
	let cluster: String = parse("CLUSTER_ID")?
		.or(raw.cluster_id.clone())
		.unwrap_or("Kubernetes".to_string());
	let xds = {
		let address = validate_uri(empty_to_none(parse("XDS_ADDRESS")?).or(raw.xds_address))?;
		// if local_config.is_none() && address.is_none() {
		// 	anyhow::bail!("file or XDS configuration is required")
		// }
		let (namespace, gateway) = if address.is_some() {
			(
				parse("NAMESPACE")?
					.or(raw.namespace.clone())
					.context("NAMESPACE is required")?,
				parse("GATEWAY")?
					.or(raw.gateway)
					.context("GATEWAY is required")?,
			)
		} else {
			("default".to_string(), "default".to_string())
		};

		let tok = parse("XDS_AUTH_TOKEN")?.or(raw.xds_auth_token);
		let auth = match tok {
			None => {
				// If nothing is set, conditionally use the default if it exists
				if Path::new(&"./var/run/secrets/xds-tokens/xds-token").exists() {
					crate::control::AuthSource::Token(
						PathBuf::from("./var/run/secrets/xds-tokens/xds-token"),
						cluster.clone(),
					)
				} else {
					crate::control::AuthSource::None
				}
			},
			Some(p) if Path::new(&p).exists() => {
				// This is a file
				crate::control::AuthSource::Token(PathBuf::from(p), cluster.clone())
			},
			Some(p) => {
				anyhow::bail!("auth token {p} not found")
			},
		};
		let xds_cert = parse_default(
			"XDS_ROOT_CA",
			"./var/run/secrets/xds/root-cert.pem".to_string(),
		)?;
		let xds_root_cert = if Path::new(&xds_cert).exists() {
			crate::control::RootCert::File(xds_cert.into())
		} else if xds_cert.eq("SYSTEM") {
			// handle SYSTEM special case for ca
			crate::control::RootCert::Default
		} else {
			crate::control::RootCert::Default
		};
		XDSConfig {
			address,
			auth,
			ca_cert: xds_root_cert,
			namespace: namespace.into(),
			gateway: gateway.into(),
			local_config,
		}
	};

	let self_addr = if !xds.namespace.is_empty() && !xds.gateway.is_empty() {
		Some(WaypointIdentity {
			gateway: xds.gateway.clone(),
			namespace: xds.namespace.clone(),
		})
	} else {
		None
	};
	let ca_address = validate_uri(empty_to_none(parse("CA_ADDRESS")?).or(raw.ca_address))?;
	let ca = if let Some(addr) = ca_address {
		let td = parse("TRUST_DOMAIN")?
			.or(raw.trust_domain)
			.unwrap_or("cluster.local".to_string());
		let ns = parse("NAMESPACE")?
			.or(raw.namespace)
			.context("NAMESPACE is required")?;
		let sa = parse("SERVICE_ACCOUNT")?
			.or(raw.service_account)
			.context("SERVICE_ACCOUNT is required")?;
		let tok = parse("CA_AUTH_TOKEN")?.or(raw.ca_auth_token);
		let auth = match tok {
			None => {
				// If nothing is set, conditionally use the default if it exists
				if Path::new(&"./var/run/secrets/tokens/istio-token").exists() {
					crate::control::AuthSource::Token(
						PathBuf::from("./var/run/secrets/tokens/istio-token"),
						cluster.clone(),
					)
				} else {
					crate::control::AuthSource::None
				}
			},
			Some(p) if Path::new(&p).exists() => {
				// This is a file
				crate::control::AuthSource::Token(PathBuf::from(p), cluster.clone())
			},
			Some(p) => {
				anyhow::bail!("auth token {p} not found")
			},
		};
		let ca_headers = parse_headers("CA_HEADER_");
		let ca_cert = parse_default(
			"CA_ROOT_CA",
			"./var/run/secrets/istio/root-cert.pem".to_string(),
		)?;
		let ca_root_cert = if Path::new(&ca_cert).exists() {
			crate::control::RootCert::File(ca_cert.into())
		} else if ca_cert.eq("SYSTEM") {
			// handle SYSTEM special case for ca
			crate::control::RootCert::Default
		} else {
			crate::control::RootCert::Default
		};
		// Build the allowed trust domains list. The local trust domain is always first.
		// ADDITIONAL_TRUST_DOMAINS is a comma-separated list of extra domains to accept.
		let mut allowed_trust_domains: Vec<Strng> = vec![td.clone().into()];
		let additional = parse("ADDITIONAL_TRUST_DOMAINS")?
			.or(raw.additional_trust_domains)
			.unwrap_or_default();
		for domain in additional.split(',') {
			let domain = domain.trim();
			if !domain.is_empty() {
				allowed_trust_domains.push(domain.into());
			}
		}
		let skip_validate_trust_domain = parse::<bool>("SKIP_VALIDATE_TRUST_DOMAIN")?
			.or(raw.skip_validate_trust_domain)
			.unwrap_or(false);
		Some(caclient::Config {
			address: addr,
			secret_ttl: Duration::from_secs(86400),
			identity: Identity::Spiffe {
				trust_domain: td.into(),
				namespace: ns.into(),
				service_account: sa.into(),
			},
			auth,
			ca_cert: ca_root_cert,
			ca_headers: ca_headers?,
			allowed_trust_domains: allowed_trust_domains.into(),
			skip_validate_trust_domain,
		})
	} else {
		None
	};
	let network = parse("NETWORK")?.or(raw.network).unwrap_or_default();

	// Self-identity for locality-aware load balancing.
	// Priority:
	//  1. Operator explicitly set LOCALITY -> Static. This is the one field WDS would otherwise
	//     fill in from the node, so it's the only real "override" signal. NETWORK and NODE_NAME
	//     are commonly set by istio-ambient/downward-API in normal k8s deploys and must not
	//     bypass WDS.
	//  2. POD_NAME+NAMESPACE known -> WDS. Standard k8s case: the control plane delivers our
	//     own Workload with locality derived from the node we're scheduled on.
	//  3. No pod identity but some env (NODE_NAME / NETWORK) -> Static with what we have.
	//     Covers non-pod deploys where WDS self-lookup isn't possible.
	let locality_env = parse::<String>("LOCALITY")?;
	let node_env =
		empty_to_none(Some(ENV.node_name.clone())).or_else(|| parse("NODE_NAME").ok().flatten());
	let pod_name =
		empty_to_none(Some(ENV.pod_name.clone())).or_else(|| parse("POD_NAME").ok().flatten());
	let pod_namespace =
		empty_to_none(Some(ENV.pod_namespace.clone())).or_else(|| parse("NAMESPACE").ok().flatten());

	let build_static = || types::discovery::Workload {
		name: pod_name.clone().unwrap_or_default().into(),
		namespace: pod_namespace.clone().unwrap_or_default().into(),
		network: network.clone().into(),
		node: node_env.clone().unwrap_or_default().into(),
		cluster_id: cluster.clone().into(),
		locality: locality_env
			.as_deref()
			.map(types::discovery::Locality::parse)
			.unwrap_or_default(),
		..Default::default()
	};

	let self_identity = if locality_env.is_some() {
		Some(types::discovery::SelfIdentitySource::Static(Arc::new(
			build_static(),
		)))
	} else if let (Some(name), Some(ns)) = (pod_name.clone(), pod_namespace.clone()) {
		Some(types::discovery::SelfIdentitySource::Wds {
			name: name.into(),
			namespace: ns.into(),
			cluster_id: cluster.clone().into(),
		})
	} else if node_env.is_some() || !network.is_empty() {
		Some(types::discovery::SelfIdentitySource::Static(Arc::new(
			build_static(),
		)))
	} else {
		None
	};
	let termination_min_deadline = parse_duration("CONNECTION_MIN_TERMINATION_DEADLINE")?
		.or(raw.connection_min_termination_deadline)
		.unwrap_or_default();
	let termination_max_deadline =
		parse_duration("CONNECTION_TERMINATION_DEADLINE")?.or(raw.connection_termination_deadline);
	let tracing_env = resolve_tracing_env_overrides()?;

	let mut otlp_headers = raw
		.tracing
		.as_ref()
		.map(|t| t.headers.clone())
		.unwrap_or_default();

	if let Some(env_headers) = tracing_env.headers.clone() {
		otlp_headers.extend(env_headers);
	}

	let otlp_protocol = tracing_env
		.protocol
		.or(raw.tracing.as_ref().map(|t| t.otlp_protocol))
		.unwrap_or_default();
	// Parse admin_addr from environment variable or config file
	let admin_addr = parse::<String>("ADMIN_ADDR")?
		.or(raw.admin_addr)
		.map(|addr| Address::new(ipv6_localhost_enabled, &addr))
		.transpose()?
		.unwrap_or(Address::Localhost(ipv6_localhost_enabled, 15000));
	// Parse stats_addr from environment variable or config file
	let stats_addr = parse::<String>("STATS_ADDR")?
		.or(raw.stats_addr)
		.map(|addr| Address::new(ipv6_localhost_enabled, &addr))
		.transpose()?
		.unwrap_or(Address::SocketAddr(SocketAddr::new(bind_wildcard, 15020)));
	// Parse readiness_addr from environment variable or config file
	let readiness_addr = parse::<String>("READINESS_ADDR")?
		.or(raw.readiness_addr)
		.map(|addr| Address::new(ipv6_localhost_enabled, &addr))
		.transpose()?
		.unwrap_or(Address::SocketAddr(SocketAddr::new(bind_wildcard, 15021)));

	let threading_mode = if parse::<String>("THREADING_MODE")?.as_deref() == Some("thread_per_core") {
		ThreadingMode::ThreadPerCore
	} else {
		ThreadingMode::default()
	};

	let session_encoder = if let Some(key) = parse::<String>("SESSION_KEY")? {
		crate::http::sessionpersistence::Encoder::aes(key.trim())?
	} else {
		match raw.session.as_ref() {
			None => crate::http::sessionpersistence::Encoder::base64(),
			Some(session) => crate::http::sessionpersistence::Encoder::aes(session.key.expose_secret())?,
		}
	};
	// Browser OIDC cookie crypto is core gateway runtime config, not per-policy input.
	let oidc_cookie_encoder = parse::<String>("OIDC_COOKIE_SECRET")?
		.map(|key| crate::http::sessionpersistence::Encoder::aes(key.trim()))
		.transpose()?;

	Ok(crate::Config {
		ipv6_enabled,
		network: network.clone().into(),
		self_identity,
		admin_addr,
		stats_addr,
		readiness_addr,
		self_addr,
		xds,
		ca,
		num_worker_threads: parse_worker_threads(raw.worker_threads)?,
		termination_min_deadline,
		threading_mode,
		backend: raw.backend,
		admin_runtime_handle: None,
		termination_max_deadline: match termination_max_deadline {
			Some(period) => period,
			None => match parse::<u64>("TERMINATION_GRACE_PERIOD_SECONDS")? {
				// We want our drain period to be less than Kubernetes, so we can use the last few seconds
				// to abruptly terminate anything remaining before Kubernetes SIGKILLs us.
				// We could just take the SIGKILL, but it is even more abrupt (TCP RST vs RST_STREAM/TLS close, etc)
				// Note: we do this in code instead of in configuration so that we can use downward API to expose this variable
				// if it is added to Kubernetes (https://github.com/kubernetes/kubernetes/pull/125746).
				Some(secs) => Duration::from_secs(cmp::max(
					if secs > 10 {
						secs - 5
					} else {
						// If the grace period is really low give less buffer
						secs - 1
					},
					1,
				)),
				None => Duration::from_secs(5),
			},
		},
		tracing: raw
			.tracing
			.clone()
			.map(|t| {
				let (otlp_endpoint, otlp_path) = normalize_tracing_endpoint(
					tracing_env.endpoint.clone().or(t.otlp_endpoint.clone()),
					t.path.clone(),
				)?;
				let endpoint = otlp_endpoint.context(
					"config.tracing requires otlpEndpoint or one of OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_TRACES_ENDPOINT, or OTEL_EXPORTER_OTLP_ENDPOINT",
				)?;
				Ok::<_, anyhow::Error>(trc::DeprecatedConfig {
					endpoint: Some(endpoint),
					headers: otlp_headers.clone(),
					protocol: otlp_protocol,

					fields: t
						.fields
						.clone()
						.map(|fields| {
							Ok::<_, anyhow::Error>(LoggingFields {
								remove: Arc::new(fields.remove.into_iter().collect()),
								add: Arc::new(
									fields
										.add
										.iter()
										.map(|(k, v)| cel::Expression::new_strict(v).map(|v| (k.clone(), Arc::new(v))))
										.collect::<Result<_, _>>()?,
								),
							})
						})
						.transpose()?
						.unwrap_or_default(),
					random_sampling: t
						.random_sampling
						.as_ref()
						.map(|c| c.0.as_str())
						.map(cel::Expression::new_strict)
						.transpose()?
						.map(Arc::new),
					client_sampling: t
						.client_sampling
						.as_ref()
						.map(|c| c.0.as_str())
						.map(cel::Expression::new_strict)
						.transpose()?
						.map(Arc::new),
					path: otlp_path.unwrap_or_else(|| "/v1/traces".to_string()),
				})
			})
			.transpose()?,
		metrics: telemetry::log::MetricsConfig {
			excluded_metrics: raw
				.metrics
				.as_ref()
				.map(|f| {
					f.remove
						.clone()
						.into_iter()
						.collect::<frozen_collections::FzHashSet<String>>()
				})
				.unwrap_or_default(),
			metric_fields: Arc::new(
				raw
					.metrics
					.and_then(|f| f.fields)
					.map(|fields| {
						Ok::<_, anyhow::Error>(MetricFields {
							add: fields
								.add
								.iter()
								.map(|(k, v)| cel::Expression::new_strict(v).map(|v| (k.clone(), Arc::new(v))))
								.collect::<Result<_, _>>()?,
						})
					})
					.transpose()?
					.unwrap_or_default(),
			),
		},
		logging: telemetry::log::Config {
			filter: raw
				.logging
				.as_ref()
				.and_then(|l| l.filter.as_ref())
				.map(cel::Expression::new_strict)
				.transpose()?
				.map(Arc::new),
			level: match raw.logging.as_ref().and_then(|l| l.level.as_ref()) {
				None => "".to_string(),
				Some(RawLoggingLevel::Single(level)) => level.to_string(),
				Some(RawLoggingLevel::List(levels)) => levels.join(","),
			},
			format: raw
				.logging
				.as_ref()
				.and_then(|l| l.format.clone())
				.unwrap_or_default(),
			fields: raw
				.logging
				.and_then(|f| f.fields)
				.map(|fields| {
					Ok::<_, anyhow::Error>(LoggingFields {
						remove: Arc::new(fields.remove.into_iter().collect()),
						add: Arc::new(
							fields
								.add
								.iter()
								.map(|(k, v)| cel::Expression::new_strict(v).map(|v| (k.clone(), Arc::new(v))))
								.collect::<Result<_, _>>()?,
						),
					})
				})
				.transpose()?
				.unwrap_or_default(),
		},
		dns: client::Config {
			resolver_cfg,
			resolver_opts,
		},
		proxy_metadata: crate::ProxyMetadata {
			instance_ip: ENV.instance_ip.clone(),
			pod_name: ENV.pod_name.clone(),
			pod_namespace: ENV.pod_namespace.clone(),
			node_name: ENV.node_name.clone(),
			role: ENV.role.clone(),
			node_id: ENV.node_id.clone(),
		},
		mcp: crate::McpConfig {
			session_ttl: raw
				.mcp
				.as_ref()
				.and_then(|m| m.session_ttl)
				.unwrap_or(crate::mcp::DEFAULT_SESSION_IDLE_TTL),
		},
		session_encoder,
		oidc_cookie_encoder,
		hbone: Arc::new(agent_hbone::Config {
			// window size: per-stream limit
			window_size: parse("HTTP2_STREAM_WINDOW_SIZE")?
				.or(raw.hbone.as_ref().and_then(|h| h.window_size))
				.unwrap_or(4u32 * 1024 * 1024),
			// connection window size: per connection.
			// Setting this to the same value as window_size can introduce deadlocks in some applications
			// where clients do not read data on streamA until they receive data on streamB.
			// If streamA consumes the entire connection window, we enter a deadlock.
			// A 4x limit should be appropriate without introducing too much potential buffering.
			connection_window_size: parse("HTTP2_CONNECTION_WINDOW_SIZE")?
				.or(raw.hbone.as_ref().and_then(|h| h.connection_window_size))
				.unwrap_or(16u32 * 1024 * 1024),
			frame_size: parse("HTTP2_FRAME_SIZE")?
				.or(raw.hbone.as_ref().and_then(|h| h.frame_size))
				.unwrap_or(1024u32 * 1024),
			pool_max_streams_per_conn: parse("POOL_MAX_STREAMS_PER_CONNECTION")?
				.or(raw.hbone.as_ref().and_then(|h| h.pool_max_streams_per_conn))
				.unwrap_or(100u16),
			pool_unused_release_timeout: parse_duration("POOL_UNUSED_RELEASE_TIMEOUT")?
				.or(
					raw
						.hbone
						.as_ref()
						.and_then(|h| h.pool_unused_release_timeout),
				)
				.unwrap_or(Duration::from_secs(60 * 5)),
		}),
	})
}

fn parse<T: FromStr>(env: &str) -> anyhow::Result<Option<T>>
where
	<T as FromStr>::Err: ToString,
{
	match env::var(env) {
		Ok(val) => val
			.parse()
			.map(|v| Some(v))
			.map_err(|e: <T as FromStr>::Err| {
				anyhow::anyhow!("invalid env var {}={} ({})", env, val, e.to_string())
			}),
		Err(_) => Ok(None),
	}
}

fn parse_default<T: FromStr>(env: &str, default: T) -> anyhow::Result<T>
where
	<T as FromStr>::Err: std::error::Error + Sync + Send,
{
	parse(env).map(|v| v.unwrap_or(default))
}

fn parse_duration(env: &str) -> anyhow::Result<Option<Duration>> {
	parse::<String>(env)?
		.map(|ds| {
			durfmt::parse(&ds).map_err(|e| anyhow::anyhow!("invalid env var {}={} ({})", env, ds, e))
		})
		.transpose()
}

pub fn empty_to_none<A: AsRef<str>>(inp: Option<A>) -> Option<A> {
	if let Some(inner) = &inp
		&& inner.as_ref().is_empty()
	{
		return None;
	}
	inp
}
// tries to parse the URI so we can fail early
fn validate_uri(uri_str: Option<String>) -> anyhow::Result<Option<String>> {
	let Some(uri_str) = uri_str else {
		return Ok(uri_str);
	};
	let uri = http::Uri::try_from(&uri_str)?;
	if uri.scheme().is_none() {
		return Ok(Some("https://".to_owned() + &uri_str));
	}
	Ok(Some(uri_str))
}

/// Parse worker threads configuration, supporting both fixed numbers and percentages
fn parse_worker_threads(cfg: Option<StringOrInt>) -> anyhow::Result<usize> {
	match parse::<String>("WORKER_THREADS")?.or_else(|| cfg.map(|cfg| cfg.0)) {
		Some(value) => {
			if let Some(percent_str) = value.strip_suffix('%') {
				// Parse as percentage
				let percent: f64 = percent_str
					.parse()
					.map_err(|e| anyhow::anyhow!("invalid percentage: {}", e))?;

				if percent <= 0.0 || percent > 100.0 {
					anyhow::bail!("percentage must be between 0 and 100".to_string())
				}

				let cpu_count = get_cpu_count()?;
				// Round up, minimum of 1
				let threads = ((cpu_count as f64 * percent / 100.0).ceil() as usize).max(1);
				Ok(threads)
			} else {
				// Parse as fixed number
				value
					.parse::<usize>()
					.map_err(|e| anyhow::anyhow!("invalid number: {}", e))
			}
		},
		None => Ok(get_cpu_count()?),
	}
}

fn parse_otlp_headers(
	env_key: &str,
) -> anyhow::Result<Option<std::collections::HashMap<String, String>>> {
	match env::var(env_key) {
		Ok(raw) => {
			let s = raw.trim();
			if s.starts_with('{') {
				let map: std::collections::HashMap<String, String> = serde_json::from_str(s)
					.map_err(|e| anyhow::anyhow!("invalid {} JSON: {}", env_key, e))?;
				Ok(Some(map))
			} else {
				let mut headers = std::collections::HashMap::new();
				for pair in s.split(',') {
					let pair = pair.trim();
					if pair.is_empty() {
						continue;
					}

					let (key, value) = pair
						.split_once('=')
						.ok_or_else(|| anyhow::anyhow!("invalid {}: expected key=value format", env_key))?;
					headers.insert(key.trim().to_string(), value.trim().to_string());
				}
				Ok(Some(headers))
			}
		},
		Err(env::VarError::NotPresent) => Ok(None),
		Err(e) => Err(anyhow::anyhow!("error reading {}: {}", env_key, e)),
	}
}

fn parse_otlp_protocol(env_key: &str) -> anyhow::Result<Option<trc::Protocol>> {
	match env::var(env_key) {
		Ok(raw) => {
			let protocol = match raw.trim().trim_matches('"').to_ascii_lowercase().as_str() {
				"grpc" => trc::Protocol::Grpc,
				"http" | "http/protobuf" => trc::Protocol::Http,
				"http/json" => {
					anyhow::bail!(
						"invalid env var {}={} (http/json is not supported; use grpc or http/protobuf)",
						env_key,
						raw
					)
				},
				_ => {
					anyhow::bail!(
						"invalid env var {}={} (expected grpc or http/protobuf)",
						env_key,
						raw
					)
				},
			};
			Ok(Some(protocol))
		},
		Err(env::VarError::NotPresent) => Ok(None),
		Err(e) => Err(anyhow::anyhow!("error reading {}: {}", env_key, e)),
	}
}

fn resolve_tracing_env_overrides() -> anyhow::Result<TracingEnvOverrides> {
	let endpoint = empty_to_none(parse::<String>("OTLP_ENDPOINT")?)
		.or(empty_to_none(parse::<String>(
			"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
		)?))
		.or(empty_to_none(parse::<String>(
			"OTEL_EXPORTER_OTLP_ENDPOINT",
		)?));
	let headers = parse_otlp_headers("OTLP_HEADERS")?
		.or(parse_otlp_headers("OTEL_EXPORTER_OTLP_TRACES_HEADERS")?)
		.or(parse_otlp_headers("OTEL_EXPORTER_OTLP_HEADERS")?);
	let protocol = parse_otlp_protocol("OTLP_PROTOCOL")?
		.or(parse_otlp_protocol("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL")?)
		.or(parse_otlp_protocol("OTEL_EXPORTER_OTLP_PROTOCOL")?);

	Ok(TracingEnvOverrides {
		endpoint,
		headers,
		protocol,
	})
}

fn normalize_tracing_endpoint(
	endpoint: Option<String>,
	path: Option<String>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
	let endpoint = validate_uri(empty_to_none(endpoint))?;
	let Some(endpoint) = endpoint else {
		return Ok((None, path));
	};

	let uri: http::Uri = endpoint.parse()?;
	let endpoint = match (uri.scheme_str(), uri.authority()) {
		(Some(scheme), Some(authority)) => format!("{scheme}://{authority}"),
		_ => endpoint,
	};
	let path = match uri.path_and_query().map(|pq| pq.as_str()) {
		Some(path_and_query) if !path_and_query.is_empty() && path_and_query != "/" => {
			Some(path_and_query.to_string())
		},
		_ => path,
	};

	Ok((Some(endpoint), path))
}

/// If the resolved config has no nameservers, fall back to defaults while
/// preserving the original resolver options. Applies the configured
/// `DnsLookupFamily` as the IP lookup strategy. When `edns0` is `Some`, it
/// overrides the resolver's EDNS0 setting; when `None`, the system-provided
/// (or default) value is preserved.
fn resolve_dns_config(
	cfg: hickory_resolver::config::ResolverConfig,
	mut opts: hickory_resolver::config::ResolverOpts,
	dns_lookup_family: DnsLookupFamily,
	ipv6_enabled: bool,
	edns0: Option<bool>,
) -> (
	hickory_resolver::config::ResolverConfig,
	hickory_resolver::config::ResolverOpts,
) {
	let resolved_cfg = if cfg.name_servers().is_empty() {
		warn!(
			"no DNS nameservers found in system config, using Google Public DNS defaults. /etc/hosts entries will still be resolved"
		);
		hickory_resolver::config::ResolverConfig::udp_and_tcp(&hickory_resolver::config::GOOGLE)
	} else {
		cfg
	};
	let nameservers: Vec<_> = resolved_cfg
		.name_servers()
		.iter()
		.map(|ns| format!("{:?}", ns))
		.collect();

	let ip_strategy = dns_lookup_family.to_lookup_strategy(ipv6_enabled);
	opts.ip_strategy = ip_strategy;
	if let Some(edns0) = edns0 {
		opts.edns0 = edns0;
	}
	info!(
		nameservers = ?nameservers,
		dns_lookup_family = ?dns_lookup_family,
		ip_strategy = ?ip_strategy,
		edns0 = opts.edns0,
		"using DNS nameservers"
	);
	(resolved_cfg, opts)
}

fn get_cpu_count() -> anyhow::Result<usize> {
	// Allow overriding the count with an env var. This can be used to pass the CPU limit on Kubernetes
	// from the downward API.
	// Note the downward API will return the total thread count ("logical cores") if no limit is set,
	// so it is really the same as num_cpus.
	// We allow num_cpus for cases its not set (not on Kubernetes, etc).
	match parse::<usize>("CPU_LIMIT")? {
		Some(limit) => Ok(limit),
		// This is *logical cores*
		None => Ok(num_cpus::get()),
	}
}

fn parse_headers(prefix: &str) -> Result<Vec<(String, String)>, anyhow::Error> {
	let mut headers = Vec::new();

	for (key, value) in env::vars() {
		let stripped_key: Option<&str> = key.strip_prefix(prefix);
		match stripped_key {
			Some(stripped_key) => {
				// Env vars are typically uppercase and often use `_` instead of `-`.
				// Normalize the suffix after `prefix` so values like
				// `CA_HEADER_AUTHORIZATION` and `CA_HEADER_X_CUSTOM_HEADER`
				// map to valid header names such as `authorization` and
				// `x-custom-header`.
				let normalized_key = stripped_key.to_ascii_lowercase().replace('_', "-");
				// attempt to parse the normalized key
				let metadata_key = http::header::HeaderName::from_str(&normalized_key)
					.map_err(|_| anyhow::anyhow!("invalid header key: {}", key))?;
				// attempt to parse the value
				http::HeaderValue::from_str(&value)
					.map_err(|_| anyhow::anyhow!("invalid header value: {}", value))?;
				headers.push((metadata_key.to_string(), value));
			},
			None => continue,
		}
	}

	Ok(headers)
}

#[cfg(test)]
mod parse_headers_tests {
	use super::*;
	use std::env;
	use std::ffi::OsString;
	use std::sync::{LazyLock, Mutex};

	static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

	struct TempEnvVar {
		key: String,
		previous: Option<OsString>,
	}

	impl TempEnvVar {
		fn set(key: &str, value: &str) -> Self {
			let previous = env::var_os(key);
			unsafe {
				env::set_var(key, value);
			}
			Self {
				key: key.to_string(),
				previous,
			}
		}
	}

	impl Drop for TempEnvVar {
		fn drop(&mut self) {
			match &self.previous {
				Some(value) => unsafe {
					env::set_var(&self.key, value);
				},
				None => unsafe {
					env::remove_var(&self.key);
				},
			}
		}
	}

	#[test]
	fn test_parse_headers_valid_header_and_normalizes_name() {
		let _guard = ENV_LOCK.lock().expect("env mutex poisoned");
		let _header = TempEnvVar::set("TEST_PARSE_HEADERS_X-Test-Header", "header-value");

		let headers = parse_headers("TEST_PARSE_HEADERS_").expect("header parsing should succeed");

		assert!(headers.contains(&("x-test-header".to_string(), "header-value".to_string())));
	}

	#[test]
	fn test_parse_headers_rejects_invalid_header_key() {
		let _guard = ENV_LOCK.lock().expect("env mutex poisoned");
		let _header = TempEnvVar::set("TEST_PARSE_HEADERS_Bad@Header", "header-value");

		let err = parse_headers("TEST_PARSE_HEADERS_").expect_err("invalid header key should fail");

		assert!(
			err
				.to_string()
				.contains("invalid header key: TEST_PARSE_HEADERS_Bad@Header")
		);
	}

	#[test]
	fn test_parse_headers_rejects_invalid_header_value() {
		let _guard = ENV_LOCK.lock().expect("env mutex poisoned");
		let _header = TempEnvVar::set("TEST_PARSE_HEADERS_X-Test-Header", "bad\nvalue");

		let err = parse_headers("TEST_PARSE_HEADERS_").expect_err("invalid header value should fail");

		assert!(err.to_string().contains("invalid header value: bad\nvalue"));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::env;
	use std::ffi::OsString;
	use std::sync::{LazyLock, Mutex};

	static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

	fn lock_env() -> std::sync::MutexGuard<'static, ()> {
		ENV_LOCK.lock().expect("env mutex poisoned")
	}

	struct TempEnvVar {
		key: String,
		previous: Option<OsString>,
	}

	impl TempEnvVar {
		fn set(key: &str, value: &str) -> Self {
			let previous = env::var_os(key);
			unsafe {
				env::set_var(key, value);
			}
			Self {
				key: key.to_string(),
				previous,
			}
		}
	}

	impl Drop for TempEnvVar {
		fn drop(&mut self) {
			match &self.previous {
				Some(value) => unsafe {
					env::set_var(&self.key, value);
				},
				None => unsafe {
					env::remove_var(&self.key);
				},
			}
		}
	}

	#[test]
	fn test_parse_otlp_headers() {
		let _env_lock = lock_env();

		unsafe {
			// Test JSON format
			env::set_var(
				"TEST_OTLP_HEADERS",
				r#"{"content-type": "application/json", "x-api-key": "secret"}"#,
			);
		}
		let json_result = parse_otlp_headers("TEST_OTLP_HEADERS").unwrap().unwrap();
		assert_eq!(
			json_result.get("content-type"),
			Some(&"application/json".to_string())
		);
		assert_eq!(json_result.get("x-api-key"), Some(&"secret".to_string()));

		unsafe {
			// Test comma-delimited format
			env::set_var(
				"TEST_OTLP_HEADERS",
				"authorization=Bearer token,x-trace-id=abc123",
			);
		}
		let comma_result = parse_otlp_headers("TEST_OTLP_HEADERS").unwrap().unwrap();
		assert_eq!(
			comma_result.get("authorization"),
			Some(&"Bearer token".to_string())
		);
		assert_eq!(comma_result.get("x-trace-id"), Some(&"abc123".to_string()));

		unsafe {
			// Test error cases
			env::set_var("TEST_OTLP_HEADERS", "{invalid json");
		}
		assert!(parse_otlp_headers("TEST_OTLP_HEADERS").is_err());

		unsafe {
			env::set_var("TEST_OTLP_HEADERS", "missing_equals");
		}
		assert!(parse_otlp_headers("TEST_OTLP_HEADERS").is_err());

		unsafe {
			env::remove_var("TEST_OTLP_HEADERS");
		}

		// Test missing env var
		assert_eq!(parse_otlp_headers("NONEXISTENT_VAR").unwrap(), None);
	}

	#[test]
	fn tracing_accepts_standard_otlp_env_vars() {
		let _env_lock = lock_env();
		let _endpoint = TempEnvVar::set(
			"OTEL_EXPORTER_OTLP_ENDPOINT",
			"http://collector.example:4318",
		);
		let _protocol = TempEnvVar::set("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf");

		let config = parse_config(
			r#"
config:
  tracing: {}
"#
			.to_string(),
			None,
		)
		.expect("config should parse");

		let tracing = config.tracing.expect("tracing config should exist");
		assert_eq!(
			tracing.endpoint.as_deref(),
			Some("http://collector.example:4318")
		);
		assert_eq!(tracing.protocol, trc::Protocol::Http);
		assert_eq!(tracing.path, "/v1/traces");
	}

	#[test]
	fn tracing_prefers_signal_specific_otlp_env_vars() {
		let _env_lock = lock_env();
		let _endpoint = TempEnvVar::set(
			"OTEL_EXPORTER_OTLP_ENDPOINT",
			"http://collector.example:4318",
		);
		let _traces_endpoint = TempEnvVar::set(
			"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
			"http://traces.example:4318/custom/traces",
		);
		let _headers = TempEnvVar::set("OTEL_EXPORTER_OTLP_HEADERS", "authorization=general");
		let _traces_headers =
			TempEnvVar::set("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "authorization=traces");
		let _protocol = TempEnvVar::set("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");
		let _traces_protocol = TempEnvVar::set("OTEL_EXPORTER_OTLP_TRACES_PROTOCOL", "http/protobuf");

		let config = parse_config(
			r#"
config:
  tracing: {}
"#
			.to_string(),
			None,
		)
		.expect("config should parse");

		let tracing = config.tracing.expect("tracing config should exist");
		assert_eq!(
			tracing.endpoint.as_deref(),
			Some("http://traces.example:4318")
		);
		assert_eq!(tracing.path, "/custom/traces");
		assert_eq!(
			tracing.headers.get("authorization"),
			Some(&"traces".to_string())
		);
		assert_eq!(tracing.protocol, trc::Protocol::Http);
	}

	#[test]
	fn tracing_prefers_legacy_otlp_env_vars() {
		let _env_lock = lock_env();
		let _legacy_endpoint = TempEnvVar::set("OTLP_ENDPOINT", "http://legacy.example:4317");
		let _standard_endpoint = TempEnvVar::set(
			"OTEL_EXPORTER_OTLP_ENDPOINT",
			"http://collector.example:4318",
		);
		let _legacy_headers = TempEnvVar::set("OTLP_HEADERS", "authorization=legacy");
		let _standard_headers = TempEnvVar::set("OTEL_EXPORTER_OTLP_HEADERS", "authorization=standard");
		let _legacy_protocol = TempEnvVar::set("OTLP_PROTOCOL", "grpc");
		let _standard_protocol = TempEnvVar::set("OTEL_EXPORTER_OTLP_PROTOCOL", "http/protobuf");

		let config = parse_config(
			r#"
config:
  tracing: {}
"#
			.to_string(),
			None,
		)
		.expect("config should parse");

		let tracing = config.tracing.expect("tracing config should exist");
		assert_eq!(
			tracing.endpoint.as_deref(),
			Some("http://legacy.example:4317")
		);
		assert_eq!(
			tracing.headers.get("authorization"),
			Some(&"legacy".to_string())
		);
		assert_eq!(tracing.protocol, trc::Protocol::Grpc);
	}

	#[test]
	fn tracing_requires_endpoint_from_config_or_env() {
		let _env_lock = lock_env();

		let err = parse_config(
			r#"
config:
  tracing: {}
"#
			.to_string(),
			None,
		)
		.expect_err("missing tracing endpoint should fail");

		assert!(
			err
				.to_string()
				.contains("config.tracing requires otlpEndpoint"),
			"unexpected error: {err}"
		);
	}

	#[test]
	fn session_key_env_overrides_inline_session_config() {
		let _env_lock = lock_env();

		let env_key = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
		let inline_key = "f1e1d1c1b1a1918171615141312111000f0e0d0c0b0a09080706050403020100";

		unsafe {
			env::set_var("SESSION_KEY", env_key);
		}

		let config = parse_config(
			format!(
				r#"
config:
  session:
    key: "{inline_key}"
"#
			),
			None,
		)
		.expect("config should parse");

		let state = crate::http::sessionpersistence::SessionState::HTTP(
			crate::http::sessionpersistence::HTTPSessionState {
				backend: "127.0.0.1:8080".parse().expect("socket addr"),
			},
		);
		let encoded = state.encode(&config.session_encoder).expect("encode state");

		let env_encoder =
			crate::http::sessionpersistence::Encoder::aes(env_key).expect("encoder from env");
		let inline_encoder =
			crate::http::sessionpersistence::Encoder::aes(inline_key).expect("inline encoder");

		assert!(crate::http::sessionpersistence::SessionState::decode(&encoded, &env_encoder).is_ok());
		assert!(
			crate::http::sessionpersistence::SessionState::decode(&encoded, &inline_encoder).is_err()
		);

		unsafe {
			env::remove_var("SESSION_KEY");
		}
	}

	#[test]
	fn resolve_dns_config_uses_defaults_when_nameservers_empty() {
		let empty_cfg = hickory_resolver::config::ResolverConfig::from_parts(None, vec![], vec![]);
		let mut custom_opts = hickory_resolver::config::ResolverOpts::default();
		custom_opts.ndots = 42;

		let (resolved_cfg, resolved_opts) = resolve_dns_config(
			empty_cfg,
			custom_opts,
			DnsLookupFamily::default(),
			true,
			None,
		);

		assert!(
			!resolved_cfg.name_servers().is_empty(),
			"should fall back to default config with nameservers"
		);
		assert_eq!(resolved_opts.ndots, 42, "should preserve original opts");
	}

	#[test]
	fn resolve_dns_config_keeps_valid_config() {
		let valid_cfg =
			hickory_resolver::config::ResolverConfig::udp_and_tcp(&hickory_resolver::config::GOOGLE);
		let mut custom_opts = hickory_resolver::config::ResolverOpts::default();
		custom_opts.ndots = 7;

		let original_count = valid_cfg.name_servers().len();
		let (resolved_cfg, resolved_opts) = resolve_dns_config(
			valid_cfg,
			custom_opts,
			DnsLookupFamily::default(),
			true,
			None,
		);

		assert_eq!(
			resolved_cfg.name_servers().len(),
			original_count,
			"should keep original nameservers"
		);
		assert_eq!(resolved_opts.ndots, 7, "should preserve original opts");
	}

	#[rstest::rstest]
	#[case(
		DnsLookupFamily::V4Only,
		true,
		hickory_resolver::config::LookupIpStrategy::Ipv4Only
	)]
	#[case(
		DnsLookupFamily::V6Only,
		false,
		hickory_resolver::config::LookupIpStrategy::Ipv6Only
	)]
	#[case(
		DnsLookupFamily::Auto,
		false,
		hickory_resolver::config::LookupIpStrategy::Ipv4Only
	)]
	#[case(
		DnsLookupFamily::Auto,
		true,
		hickory_resolver::config::LookupIpStrategy::Ipv4thenIpv6
	)]
	fn resolve_dns_config_ip_strategy(
		#[case] family: DnsLookupFamily,
		#[case] ipv6_enabled: bool,
		#[case] expected: hickory_resolver::config::LookupIpStrategy,
	) {
		let cfg = hickory_resolver::config::ResolverConfig::default();
		let opts = hickory_resolver::config::ResolverOpts::default();

		let (_, resolved_opts) = resolve_dns_config(cfg, opts, family, ipv6_enabled, None);

		assert_eq!(resolved_opts.ip_strategy, expected);
	}

	#[rstest::rstest]
	#[case(false, None, false)]
	#[case(false, Some(true), true)]
	#[case(true, Some(false), false)]
	fn resolve_dns_config_edns0(
		#[case] initial_edns0: bool,
		#[case] edns0_param: Option<bool>,
		#[case] expected: bool,
	) {
		let cfg = hickory_resolver::config::ResolverConfig::default();
		let mut opts = hickory_resolver::config::ResolverOpts::default();
		opts.edns0 = initial_edns0;

		let (_, resolved_opts) =
			resolve_dns_config(cfg, opts, DnsLookupFamily::default(), true, edns0_param);

		assert_eq!(resolved_opts.edns0, expected);
	}

	#[test]
	fn session_key_env_enables_aes_session_encoder() {
		let _env_lock = lock_env();

		let session_key = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

		unsafe {
			env::set_var("SESSION_KEY", session_key);
		}

		let config = parse_config("{}".to_string(), None).expect("config should parse");
		assert!(matches!(
			config.session_encoder,
			crate::http::sessionpersistence::Encoder::Aes(_)
		));

		unsafe {
			env::remove_var("SESSION_KEY");
		}
	}
}
