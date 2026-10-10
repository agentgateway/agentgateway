use ::http::Uri;
use hyper_util::client::proxy::matcher::{Intercept, Matcher};

use crate::client::{ApplicationTransport, Client, Transport, TunnelConfig};
use crate::types::agent::Target;
use crate::*;

/// Proxy for calls the gateway makes on its own behalf (credential fetches, JWKS, ext_authz, etc).
#[derive(Debug)]
pub(super) struct Proxy {
	matcher: Matcher,
	http: Option<TunnelConfig>,
	https: Option<TunnelConfig>,
}

impl Client {
	/// Configures the callout proxy from `config.callouts.tunnel`, falling back to the standard
	/// `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY` environment variables.
	pub fn with_callouts(mut self, cfg: &crate::CalloutConfig) -> anyhow::Result<Client> {
		let (all, http, https, no) = match &cfg.tunnel {
			Some(tunnel) => {
				if !matches!(tunnel.url.scheme_str(), Some("http" | "https")) {
					anyhow::bail!("callout tunnel url must use http or https");
				}
				(
					tunnel.url.to_string(),
					String::new(),
					String::new(),
					tunnel.no_proxy.join(","),
				)
			},
			None => (
				env(&["ALL_PROXY", "all_proxy"]),
				env(&["HTTP_PROXY", "http_proxy"]),
				env(&["HTTPS_PROXY", "https_proxy"]),
				env(&["NO_PROXY", "no_proxy"]),
			),
		};
		// Matcher does not expose its proxies directly. Without a NO_PROXY list, `intercept` returns
		// the proxy for the destination's scheme regardless of host, which lets us extract them.
		let proxies = Matcher::builder()
			.all(&all)
			.http(&http)
			.https(&https)
			.build();
		let proxy_for = |dst: &'static str| {
			proxies
				.intercept(&Uri::from_static(dst))
				.map(|i| tunnel_config(&i))
				.transpose()
		};
		let http_tunnel = proxy_for("http://callout")?;
		let https_tunnel = proxy_for("https://callout")?;
		if http_tunnel.is_none() && https_tunnel.is_none() {
			return Ok(self);
		}
		let matcher = Matcher::builder()
			.all(all)
			.http(http)
			.https(https)
			.no(no)
			.build();
		info!("callouts will use proxy {matcher:?}");
		self.callout_proxy = Some(Arc::new(Proxy {
			matcher,
			http: http_tunnel,
			https: https_tunnel,
		}));
		Ok(self)
	}

	/// Sends a callout through the callout proxy, if one is configured and applies to the target.
	/// Customized transports (such as an explicit tunnel or HBONE) are left as is.
	pub fn callout_transport(&self, transport: Transport, target: &Target) -> Transport {
		let (Some(proxy), Transport::Plain(app)) = (&self.callout_proxy, &transport) else {
			return transport;
		};
		let authority = match target {
			Target::Hostname(host, port) => format!("{host}:{port}"),
			// Loopback and link-local destinations (such as cloud metadata servers) are never reachable
			// through a remote proxy.
			Target::Address(addr) if addr.ip().is_loopback() || is_link_local(addr.ip()) => {
				return transport;
			},
			Target::Address(addr) => addr.to_string(),
			Target::UnixSocket(_) => return transport,
		};
		let (scheme, tunnel) = match app {
			ApplicationTransport::Tls(_) => ("https", &proxy.https),
			ApplicationTransport::Plaintext => ("http", &proxy.http),
		};
		let Some(tunnel) = tunnel else {
			return transport;
		};
		let intercepted = Uri::try_from(format!("{scheme}://{authority}"))
			.is_ok_and(|uri| proxy.matcher.intercept(&uri).is_some());
		if !intercepted {
			return transport;
		}
		Transport::Tunnel(app.clone(), tunnel.clone())
	}
}

fn tunnel_config(proxy: &Intercept) -> anyhow::Result<TunnelConfig> {
	let uri = proxy.uri();
	let tls = match uri.scheme_str() {
		Some("http") => false,
		Some("https") => true,
		_ => anyhow::bail!("callout proxy {uri} must use http or https"),
	};
	let host = uri
		.host()
		.ok_or_else(|| anyhow::anyhow!("callout proxy {uri} must have a host"))?
		.trim_start_matches('[')
		.trim_end_matches(']');
	let port = uri.port_u16().unwrap_or(if tls { 443 } else { 80 });
	let app = if tls {
		ApplicationTransport::Tls(http::backendtls::SYSTEM_TRUST.base_config())
	} else {
		ApplicationTransport::Plaintext
	};
	Ok(TunnelConfig {
		target: Target::from((host, port)),
		connection: Box::new(Transport::Plain(app).into()),
		token: proxy.basic_auth().cloned(),
		connect_headers: vec![],
		connect: false,
	})
}

fn env(names: &[&str]) -> String {
	names
		.iter()
		.find_map(|name| std::env::var(name).ok())
		.unwrap_or_default()
}

fn is_link_local(ip: IpAddr) -> bool {
	match ip {
		IpAddr::V4(ip) => ip.is_link_local(),
		IpAddr::V6(ip) => ip.is_unicast_link_local(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn callout_transport() {
		let client = crate::test_helpers::test_client()
			.with_callouts(&crate::CalloutConfig {
				tunnel: Some(crate::CalloutTunnel {
					url: "http://user:pass@proxy.example:3128".parse().unwrap(),
					// `callout` would collide with a placeholder host used to extract the proxies.
					no_proxy: vec!["callout".into(), "internal.example".into()],
				}),
			})
			.unwrap();
		let tls = || {
			Transport::Plain(ApplicationTransport::Tls(
				http::backendtls::SYSTEM_TRUST.base_config(),
			))
		};
		let host = |h: &str, port| Target::Hostname(h.into(), port);

		let Transport::Tunnel(_, tunnel) = client.callout_transport(tls(), &host("idp.example", 443))
		else {
			panic!("expected tunnel");
		};
		assert_eq!(tunnel.target, host("proxy.example", 3128));
		assert_eq!(tunnel.token.unwrap(), "Basic dXNlcjpwYXNz");

		let direct = [
			client.callout_transport(tls(), &host("api.internal.example", 443)),
			client.callout_transport(
				tls(),
				&Target::Address("169.254.169.254:80".parse().unwrap()),
			),
			client.callout_transport(tls(), &Target::Address("127.0.0.1:80".parse().unwrap())),
		];
		for t in direct {
			assert!(matches!(t, Transport::Plain(_)), "{t:?}");
		}
	}
}
