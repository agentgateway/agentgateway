use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{cmp, env};

use agent_core::durfmt;
use agent_core::prelude::*;
use serde::de::DeserializeOwned;

use crate::control::caclient;
use crate::telemetry::log::{LoggingFields, MetricFields};
use crate::telemetry::trc;
use crate::types::discovery::Identity;
use crate::{
	Address, Config, ConfigSource, NestedRawConfig, RawLoggingLevel, StringOrInt, ThreadingMode,
	XDSConfig, cel, client, serdes, telemetry,
};

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

	let (resolver_cfg, resolver_opts) = hickory_resolver::system_conf::read_system_conf()?;

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
			("".to_string(), "".to_string())
		};
		let cluster: String = parse("CLUSTER_ID")?
			.or(raw.cluster_id.clone())
			.unwrap_or("Kubernetes".to_string());
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
			namespace,
			gateway,
			local_config,
		}
	};

	let self_addr = if !xds.namespace.is_empty() && !xds.gateway.is_empty() {
		// TODO: this is bad
		Some(strng::format!(
			"{}.{}.svc.cluster.local",
			xds.gateway,
			xds.namespace
		))
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
		let cluster: String = parse("CLUSTER_ID")?
			.or(raw.cluster_id)
			.unwrap_or("Kubernetes".to_string());
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
		})
	} else {
		None
	};
	let network = parse("NETWORK")?.or(raw.network).unwrap_or_default();
	let termination_min_deadline = parse_duration("CONNECTION_MIN_TERMINATION_DEADLINE")?
		.or(raw.connection_min_termination_deadline)
		.unwrap_or_default();
	let termination_max_deadline =
		parse_duration("CONNECTION_TERMINATION_DEADLINE")?.or(raw.connection_termination_deadline);
	let otlp = empty_to_none(parse("OTLP_ENDPOINT")?)
		.or(raw.tracing.as_ref().map(|t| t.otlp_endpoint.clone()));

	let mut otlp_headers = raw
		.tracing
		.as_ref()
		.map(|t| t.headers.clone())
		.unwrap_or_default();

	if let Some(env_headers) = parse_otlp_headers("OTLP_HEADERS")? {
		otlp_headers.extend(env_headers);
	}

	let otlp_protocol = parse_serde("OTLP_PROTOCOL")?
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

	Ok(crate::Config {
		network: network.into(),
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
		tracing: trc::Config {
			endpoint: otlp,
			headers: otlp_headers,
			protocol: otlp_protocol,

			fields: Arc::new(
				raw
					.tracing
					.as_ref()
					.and_then(|f| f.fields.clone())
					.map(|fields| {
						Ok::<_, anyhow::Error>(LoggingFields {
							remove: fields.remove.into_iter().collect(),
							add: fields
								.add
								.iter()
								.map(|(k, v)| cel::Expression::new(v).map(|v| (k.clone(), Arc::new(v))))
								.collect::<Result<_, _>>()?,
						})
					})
					.transpose()?
					.unwrap_or_default(),
			),
			random_sampling: raw
				.tracing
				.as_ref()
				.and_then(|t| t.random_sampling.as_ref().map(|c| c.0.as_str()))
				.map(cel::Expression::new)
				.transpose()?
				.map(Arc::new),
			client_sampling: raw
				.tracing
				.as_ref()
				.and_then(|t| t.client_sampling.as_ref().map(|c| c.0.as_str()))
				.map(cel::Expression::new)
				.transpose()?
				.map(Arc::new),
		},
		logging: telemetry::log::Config {
			filter: raw
				.logging
				.as_ref()
				.and_then(|l| l.filter.as_ref())
				.map(cel::Expression::new)
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
			fields: Arc::new(
				raw
					.logging
					.and_then(|f| f.fields)
					.map(|fields| {
						Ok::<_, anyhow::Error>(LoggingFields {
							remove: fields.remove.into_iter().collect(),
							add: fields
								.add
								.iter()
								.map(|(k, v)| cel::Expression::new(v).map(|v| (k.clone(), Arc::new(v))))
								.collect::<Result<_, _>>()?,
						})
					})
					.transpose()?
					.unwrap_or_default(),
			),
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
								.map(|(k, v)| cel::Expression::new(v).map(|v| (k.clone(), Arc::new(v))))
								.collect::<Result<_, _>>()?,
						})
					})
					.transpose()?
					.unwrap_or_default(),
			),
		},
		dns: client::Config {
			// TODO: read from file
			resolver_cfg,
			resolver_opts,
		},
		proxy_metadata: crate::ProxyMetadata {
			instance_ip: std::env::var("INSTANCE_IP").unwrap_or_else(|_| "1.1.1.1".to_string()),
			pod_name: std::env::var("POD_NAME").unwrap_or_else(|_| "".to_string()),
			pod_namespace: std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "".to_string()),
			node_name: std::env::var("NODE_NAME").unwrap_or_else(|_| "".to_string()),
			role: format!(
				"{ns}~{name}",
				ns = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "".to_string()),
				name = std::env::var("GATEWAY").unwrap_or_else(|_| "".to_string())
			),
			node_id: format!(
				"agentgateway~{ip}~{pod_name}.{ns}~{ns}.svc.cluster.local",
				ip = std::env::var("INSTANCE_IP").unwrap_or_else(|_| "1.1.1.1".to_string()),
				pod_name = std::env::var("POD_NAME").unwrap_or_else(|_| "".to_string()),
				ns = std::env::var("POD_NAMESPACE").unwrap_or_else(|_| "".to_string())
			),
		},
		listener: raw.listener,
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

fn parse_serde<T: DeserializeOwned>(env: &str) -> anyhow::Result<Option<T>> {
	match env::var(env) {
		Ok(val) => serde_json::from_str(&val)
			.map(|v| Some(v))
			.map_err(|e| anyhow::anyhow!("invalid env var {}={} ({})", env, val, e)),
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_parse_otlp_headers() {
		use std::env;

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
}
