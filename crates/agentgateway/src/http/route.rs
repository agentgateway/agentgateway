use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::strng;

use crate::http::Request;
use crate::types::agent;
use crate::types::agent::{
	BackendReference, HeaderMatch, HeaderValueMatch, Listener, PathMatch, QueryValueMatch, Route,
	RouteBackendReference, RouteMatch, RouteName,
};
use crate::*;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "route_test.rs"]
mod tests;

/// Check if a RouteMatch matches the given request (path, method, headers, query).
fn matches_request(m: &RouteMatch, request: &Request) -> bool {
	let path_matches = match &m.path {
		PathMatch::Exact(p) => request.uri().path() == p.as_str(),
		PathMatch::Regex(r) => {
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
		let Some(have) = http::get_pseudo_or_header_value(name, request) else {
			return false;
		};
		match value {
			HeaderValueMatch::Exact(want) => {
				if have.as_ref() != *want {
					return false;
				}
			},
			HeaderValueMatch::Regex(want) => {
				let Some(have_str) = have.to_str().ok() else {
					return false;
				};
				let Some(m) = want.find(have_str) else {
					return false;
				};
				if !(m.start() == 0 && m.end() == have_str.len()) {
					return false;
				}
			},
		}
	}
	// TODO: this re-parses the query string on every call; hoist to caller if this becomes a hot path.
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
				let Some(m) = want.find(have) else {
					return false;
				};
				if !(m.start() == 0 && m.end() == have.len()) {
					return false;
				}
			},
		}
	}
	true
}

pub fn select_best_route(
	stores: Stores,
	dst: SocketAddr,
	listener: &Listener,
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

	let (default_response, host) =
		if let Some(wps) = request.extensions().get::<crate::proxy::WaypointService>() {
			// When routes are attached to a Service via parentRef, they take priority
			// over listener-attached routes. If service routes exist but none match,
			// the request is rejected (per GAMMA spec).
			let svc = wps.as_ref();
			let svc_nh = svc.namespaced_hostname();
			let (has_svc_routes, svc_route_match) = {
				let binds = stores.read_binds();
				match binds.get_service_routes(&svc_nh) {
					Some(svc_routes) => {
						let mut result = None;
						for hnm in agent::HostnameMatch::all_matches(&svc.hostname) {
							result = svc_routes
								.get_hostname(&hnm)
								.find(|(_, m)| matches_request(m, request))
								.map(|(route, matcher)| (route, matcher.path.clone()));
							if result.is_some() {
								break;
							}
						}
						(true, result)
					},
					None => (false, None),
				}
			};
			if let Some(result) = svc_route_match {
				return Some(result);
			}
			if has_svc_routes {
				// GAMMA: service routes exist but none matched -> reject
				return None;
			}

			// No service-keyed routes: fall through to hostname matching with default route
			let default_route = Route {
				key: strng::literal!("_waypoint-default"),
				service_key: None,
				name: RouteName {
					name: strng::literal!("_waypoint-default"),
					namespace: svc.namespace.clone(),
					rule_name: None,
					kind: None,
				},
				hostnames: vec![],
				matches: vec![],
				inline_policies: vec![],
				backends: vec![RouteBackendReference {
					weight: 1,
					backend: BackendReference::Service {
						name: svc.namespaced_hostname(),
						port: dst.port(), // TODO: get from req
					},
					inline_policies: Vec::new(),
				}],
			};
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
		let best_match = candidates.find(|(_, m)| matches_request(m, request));
		if let Some((route, matcher)) = best_match {
			return Some((route, matcher.path.clone()));
		}
	}
	default_response
}
