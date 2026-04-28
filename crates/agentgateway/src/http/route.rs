use std::borrow::Cow;
use std::cmp;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::strng;
use itertools::Itertools;

use crate::http::Request;
use crate::proxy::dtrace;
use crate::types::agent;
use crate::types::agent::{
	BackendReference, HeaderMatch, HeaderValueMatch, Listener, PathMatch, QueryValueMatch, Route,
	RouteBackendReference, RouteMatch, RouteName, RouteSet,
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
		PathMatch::Invalid => false,
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
			HeaderValueMatch::Invalid => return false,
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
			QueryValueMatch::Invalid => return false,
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
			let svc_routes = {
				let binds = stores.read_binds();
				binds.get_service_routes(&svc_nh)
			};
			let (has_svc_routes, svc_route_match) = match svc_routes {
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
				service_key: Some(svc.namespaced_hostname()),
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
					target: BackendReference::Service {
						name: svc.namespaced_hostname(),
						port: dst.port(), // TODO: get from req
					}
					.into(),
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
	let listener_routes = {
		let binds = stores.read_binds();
		binds.get_listener_routes(&listener.key)
	};

	if let Some(routes) = listener_routes {
		let get_all = || {
			let mut all_routes = HashSet::new();
			for hnm in agent::HostnameMatch::all_matches(&host) {
				routes.get_hostname(&hnm).for_each(|i| {
					all_routes.insert(i.0.key.clone());
				});
			}
			all_routes.into_iter().sorted().collect_vec()
		};
		for hnm in agent::HostnameMatch::all_matches(&host) {
			let mut candidates = routes.get_hostname(&hnm);
			let best_match = candidates.find(|(_, m)| matches_request(m, request));
			if let Some((route, matcher)) = best_match {
				dtrace::trace(|d| {
					let selected = route.key.clone();
					d.route_selection(Some(selected), get_all());
				});
				return Some((route, matcher.path.clone()));
			}
		}
		dtrace::trace(|d| {
			d.route_selection(None, get_all());
		});
	}
	default_response
}

pub fn select_best_route_group(
	rg: &RouteSet,
	request: &Request,
) -> Option<(Arc<Route>, PathMatch)> {
	let host = http::get_host(request).ok()?;
	for hnm in agent::HostnameMatch::all_matches(host) {
		let mut candidates = rg.get_hostname(&hnm);
		let best_match = candidates.find(|(_, m)| matches_request(m, request));
		if let Some((route, matcher)) = best_match {
			return Some((route, matcher.path.clone()));
		}
	}
	None
}

pub fn best_match_for_route(route: &Route, request: &Request) -> Option<PathMatch> {
	let mut best: Option<&agent::RouteMatch> = None;
	for candidate in route.matches.iter().filter(|m| matches_request(m, request)) {
		if let Some(current) = best {
			if compare_route_match(candidate, current) == cmp::Ordering::Greater {
				best = Some(candidate);
			}
		} else {
			best = Some(candidate);
		}
	}
	best.map(|m| m.path.clone())
}

fn compare_route_match(a: &agent::RouteMatch, b: &agent::RouteMatch) -> cmp::Ordering {
	let path_rank1 = get_path_rank(&a.path);
	let path_rank2 = get_path_rank(&b.path);
	if path_rank1 != path_rank2 {
		return path_rank1.cmp(&path_rank2);
	}

	let path_len1 = get_path_length(&a.path);
	let path_len2 = get_path_length(&b.path);
	if path_len1 != path_len2 {
		return path_len1.cmp(&path_len2);
	}

	let method1 = a.method.is_some();
	let method2 = b.method.is_some();
	if method1 != method2 {
		return method1.cmp(&method2);
	}

	let header_count1 = a.headers.len();
	let header_count2 = b.headers.len();
	if header_count1 != header_count2 {
		return header_count1.cmp(&header_count2);
	}

	a.query.len().cmp(&b.query.len())
}

fn get_path_rank(path: &PathMatch) -> u8 {
	match path {
		PathMatch::Exact(_) => 3,
		PathMatch::PathPrefix(_) => 2,
		PathMatch::Regex(_) => 1,
		// Because this can never match, its rank is irrelevant
		PathMatch::Invalid => 0,
	}
}

fn get_path_length(path: &PathMatch) -> usize {
	match path {
		PathMatch::Exact(p) | PathMatch::PathPrefix(p) => p.len(),
		PathMatch::Regex(r) => r.as_str().len(),
		// Because this can never match, its rank is irrelevant
		PathMatch::Invalid => 0,
	}
}
