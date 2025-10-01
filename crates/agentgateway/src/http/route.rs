use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::strng;

use crate::http::Request;
use crate::types::agent;
use crate::types::agent::{
	BackendReference, HeaderMatch, HeaderValueMatch, Listener, ListenerProtocol, PathMatch,
	QueryValueMatch, Route, RouteBackendReference,
};
use crate::types::discovery::gatewayaddress::Destination;
use crate::types::discovery::{NamespacedHostname, NetworkAddress};
use crate::*;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "route_test.rs"]
mod tests;

pub fn select_best_route(
	stores: Stores,
	network: Strng,
	self_addr: Option<Strng>,
	dst: SocketAddr,
	listener: Arc<Listener>,
	request: &Request,
) -> Option<(Arc<Route>, PathMatch)> {
	// Order:
	// * "Exact" path match.
	// * "Prefix" path match with largest number of characters.
	// * Method match.
	// * Largest number of header matches.
	// * Largest number of query param matches.
	//
	// If ties still exist across multiple Routes, matching precedence MUST be
	// determined in order of the following criteria, continuing on ties:
	//
	//  * The oldest Route based on creation timestamp.
	//  * The Route appearing first in alphabetical order by "{namespace}/{name}".
	//
	// If ties still exist within an HTTPRoute, matching precedence MUST be granted
	// to the FIRST matching rule (in list order) with a match meeting the above
	// criteria.

	let host = http::get_host(request).ok()?;
	// TODO: ensure we actually serve this service
	let (default_response, host) = if matches!(listener.protocol, ListenerProtocol::HBONE) {
		let Some(self_addr) = self_addr else {
			warn!("waypoint requires self address");
			return None;
		};
		// We are going to get a VIP request. Look up the Service
		let svc = stores
			.read_discovery()
			.services
			.get_by_vip(&NetworkAddress {
				network,
				address: dst.ip(),
			})?;
		let wp = svc.waypoint.as_ref()?;
		// Make sure the service is actually bound to us. TODO: should we have a more explicit setup?
		match &wp.destination {
			Destination::Address(aadr) => {
				// TODO: this is pretty sketchy
				let Some(ns) = self_addr.split(".").nth(1) else {
					warn!("waypoint cannot find self namespace");
					return None;
				};
				let self_svc =
					stores
						.read_discovery()
						.services
						.get_by_namespaced_host(&NamespacedHostname {
							namespace: ns.into(),
							hostname: self_addr,
						})?;
				if !self_svc.vips.contains(aadr) {
					warn!(
						"service {} is meant for waypoint {}, but we are {:?}",
						svc.hostname, aadr, self_svc.vips,
					);
				}
			},
			Destination::Hostname(n) => {
				if n.hostname != self_addr {
					warn!(
						"service {} is meant for waypoint {}, but we are {}",
						svc.hostname, n.hostname, self_addr
					);
					return None;
				}
			},
		}
		// TODO: only build this if we don't match one
		let default_route = Route {
			key: strng::new("waypoint-default"),
			route_name: strng::new("waypoint-default"),
			rule_name: None,
			hostnames: vec![],
			matches: vec![],
			filters: vec![],
			inline_policies: vec![],
			policies: None,
			backends: vec![RouteBackendReference {
				weight: 1,
				backend: BackendReference::Service {
					name: svc.namespaced_hostname(),
					port: dst.port(), // TODO: get from req
				},
				filters: Vec::new(),
			}],
		};
		// If there is no route, use a default one
		let def = Some((
			Arc::new(default_route),
			PathMatch::PathPrefix(strng::new("/")),
		));
		(def, Cow::Owned(svc.hostname.to_string()))
	} else {
		(None, Cow::Borrowed(host))
	};
	for hnm in agent::HostnameMatch::all_matches(&host) {
		let mut candidates = listener.routes.get_hostname(&hnm);
		let best_match = candidates.find(|(_, m)| {
			let path_matches = match &m.path {
				PathMatch::Exact(p) => request.uri().path() == p.as_str(),
				PathMatch::Regex(r, _) => {
					// Regex has no defined ordering. We will order by the length of the regex expression.
					let path = request.uri().path();
					r.find(path)
						.map(|m| m.start() == 0 && m.end() == path.len())
						.unwrap_or(false)
				},
				PathMatch::PathPrefix(p) => {
					let p = p.trim_end_matches('/');
					let Some(suffix) = request.uri().path().trim_end_matches('/').strip_prefix(p) else {
						return false;
					};
					// TODO this is not right!!
					suffix.is_empty() || suffix.starts_with('/')
				},
			};
			if !path_matches {
				return false;
			}

			if let Some(method) = &m.method
				&& request.method().as_str() != method.method.as_str()
			{
				return false;
			}
			for HeaderMatch { name, value } in &m.headers {
				let Some(have) = request.headers().get(name.as_str()) else {
					return false;
				};
				match value {
					HeaderValueMatch::Exact(want) => {
						if have != want {
							return false;
						}
					},
					HeaderValueMatch::Regex(want) => {
						// Must be a valid string to do regex match
						let Some(have) = have.to_str().ok() else {
							return false;
						};
						let Some(m) = want.find(have) else {
							return false;
						};
						// Make sure we matched the entire thing
						if !(m.start() == 0 && m.end() == have.len()) {
							return false;
						}
					},
				}
			}
			let query = request
				.uri()
				.query()
				.map(|q| url::form_urlencoded::parse(q.as_bytes()).collect::<HashMap<_, _>>())
				.unwrap_or_default();
			for agent::QueryMatch { name, value } in &m.query {
				let Some(have) = query.get(name.as_str()) else {
					return false;
				};

				match value {
					QueryValueMatch::Exact(want) => {
						if have.as_ref() != want.as_str() {
							return false;
						}
					},
					QueryValueMatch::Regex(want) => {
						// Must be a valid string to do regex match
						let Some(m) = want.find(have) else {
							return false;
						};
						// Make sure we matched the entire thing
						if !(m.start() == 0 && m.end() == have.len()) {
							return false;
						}
					},
				}
			}
			true
		});
		if let Some((route, matcher)) = best_match {
			return Some((route, matcher.path.clone()));
		}
	}
	default_response
}
