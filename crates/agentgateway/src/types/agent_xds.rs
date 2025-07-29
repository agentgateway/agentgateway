use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::Display;
use std::io::Cursor;
use std::marker::PhantomData;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::str::FromStr;
use std::sync::Arc;
use std::{cmp, net};

use anyhow::anyhow;
use duration_str::DError::ParseError;
use indexmap::IndexMap;
use itertools::Itertools;
use once_cell::sync::Lazy;
use openapiv3::OpenAPI;
use regex::Regex;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, ServerConfig};
use rustls_pemfile::Item;
use secrecy::SecretString;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use super::agent::*;
use crate::http::auth::BackendAuth;
use crate::http::jwt::Jwt;
use crate::http::localratelimit::RateLimit;
use crate::http::{
	HeaderName, HeaderValue, StatusCode, ext_proc, filters, localratelimit, retry, status, timeout,
	uri,
};
use crate::llm::{AIBackend, AIProvider};
use crate::mcp::rbac::RuleSet;
use crate::transport::tls;
use crate::types::agent::Backend::Opaque;
use crate::types::discovery::NamespacedHostname;
use crate::types::proto;
use crate::types::proto::ProtoError;
use crate::types::proto::agent::mcp_target::Protocol;
use crate::types::proto::agent::policy_spec::ExternalAuth;
use crate::types::proto::agent::policy_spec::local_rate_limit::Type;
use crate::*;

impl TryFrom<&proto::agent::TlsConfig> for TLSConfig {
	type Error = anyhow::Error;

	fn try_from(value: &proto::agent::TlsConfig) -> Result<Self, Self::Error> {
		let cert_chain = parse_cert(&value.cert)?;
		let private_key = parse_key(&value.private_key)?;
		let mut sc = ServerConfig::builder_with_provider(transport::tls::provider())
			.with_protocol_versions(transport::tls::ALL_TLS_VERSIONS)
			.expect("server config must be valid")
			.with_no_client_auth()
			.with_single_cert(cert_chain, private_key)?;
		// TODO: support h2
		sc.alpn_protocols = vec![b"http/1.1".into()];
		Ok(TLSConfig {
			config: Arc::new(sc),
		})
	}
}

impl TryFrom<&proto::agent::RouteBackend> for RouteBackendReference {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::RouteBackend) -> Result<Self, Self::Error> {
		let kind = resolve_reference(s.backend.as_ref())?;
		let filters = s
			.filters
			.iter()
			.map(RouteFilter::try_from)
			.collect::<Result<Vec<_>, _>>()?;
		Ok(Self {
			weight: s.weight as usize,
			backend: kind,
			filters,
		})
	}
}

impl TryFrom<proto::agent::TrafficPolicy> for TrafficPolicy {
	type Error = ProtoError;

	fn try_from(s: proto::agent::TrafficPolicy) -> Result<Self, Self::Error> {
		let req = s.request_timeout.map(|v| v.try_into()).transpose()?;
		let backend = s
			.backend_request_timeout
			.map(|v| v.try_into())
			.transpose()?;

		Ok(Self {
			timeout: crate::http::timeout::Policy {
				request_timeout: req,
				backend_request_timeout: backend,
			},
			// TODO: pass in XDS
			retry: None,
		})
	}
}

impl TryFrom<(proto::agent::Protocol, Option<&proto::agent::TlsConfig>)> for ListenerProtocol {
	type Error = ProtoError;
	fn try_from(
		value: (proto::agent::Protocol, Option<&proto::agent::TlsConfig>),
	) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::Protocol;
		match (value.0, value.1) {
			(Protocol::Unknown, _) => Err(ProtoError::EnumParse("unknown protocol".into())),
			(Protocol::Http, None) => Ok(ListenerProtocol::HTTP),
			(Protocol::Https, Some(tls)) => Ok(ListenerProtocol::HTTPS(
				tls
					.try_into()
					.map_err(|e| ProtoError::Generic(format!("{e}")))?,
			)),
			(Protocol::Tls, Some(tls)) => Ok(ListenerProtocol::TLS(
				tls
					.try_into()
					.map_err(|e| ProtoError::Generic(format!("{e}")))?,
			)),
			(Protocol::Tcp, None) => Ok(ListenerProtocol::TCP),
			(Protocol::Hbone, None) => Ok(ListenerProtocol::HBONE),
			(proto, tls) => Err(ProtoError::Generic(format!(
				"protocol {:?} is incompatible with {}",
				proto,
				if tls.is_some() {
					"tls"
				} else {
					"no tls config"
				}
			))),
		}
	}
}

impl TryFrom<&proto::agent::Bind> for Bind {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Bind) -> Result<Self, Self::Error> {
		Ok(Self {
			key: s.key.clone().into(),
			address: SocketAddr::from((IpAddr::from([0, 0, 0, 0]), s.port as u16)),
			listeners: Default::default(),
		})
	}
}

impl TryFrom<&proto::agent::Listener> for (Listener, BindName) {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Listener) -> Result<Self, Self::Error> {
		let proto = proto::agent::Protocol::try_from(s.protocol)?;
		let protocol = ListenerProtocol::try_from((proto, s.tls.as_ref()))
			.map_err(|e| ProtoError::Generic(format!("{e}")))?;
		let l = Listener {
			key: strng::new(&s.key),
			name: strng::new(&s.name),
			hostname: s.hostname.clone().into(),
			protocol,
			gateway_name: strng::new(&s.gateway_name),
			routes: Default::default(),
			tcp_routes: Default::default(),
		};
		Ok((l, strng::new(&s.bind_key)))
	}
}

impl TryFrom<&proto::agent::Route> for (Route, ListenerKey) {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Route) -> Result<Self, Self::Error> {
		let r = Route {
			key: strng::new(&s.key),
			route_name: strng::new(&s.route_name),
			rule_name: default_as_none(s.rule_name.as_str()).map(strng::new),
			hostnames: s.hostnames.iter().map(strng::new).collect(),
			// TODO
			matches: s
				.matches
				.iter()
				.map(RouteMatch::try_from)
				.collect::<Result<Vec<_>, _>>()?,
			filters: s
				.filters
				.iter()
				.map(RouteFilter::try_from)
				.collect::<Result<Vec<_>, _>>()?,
			backends: s
				.backends
				.iter()
				.map(RouteBackendReference::try_from)
				.collect::<Result<Vec<_>, _>>()?,
			policies: s.traffic_policy.map(TrafficPolicy::try_from).transpose()?,
		};
		Ok((r, strng::new(&s.listener_key)))
	}
}

impl TryFrom<&proto::agent::Backend> for Backend {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Backend) -> Result<Self, Self::Error> {
		let name = BackendName::from(&s.name);
		Ok(match &s.kind {
			Some(proto::agent::backend::Kind::Static(s)) => Backend::Opaque(
				name,
				Target::try_from((s.host.as_str(), s.port as u16))
					.map_err(|e| ProtoError::Generic(e.to_string()))?,
			),
			Some(proto::agent::backend::Kind::Ai(a)) => Backend::AI(
				name,
				AIBackend {
					host_override: a
						.r#override
						.as_ref()
						.map(|o| {
							Target::try_from((o.host.as_str(), o.port as u16))
								.map_err(|e| ProtoError::Generic(e.to_string()))
						})
						.transpose()?,
					provider: match &a.provider {
						Some(proto::agent::ai_backend::Provider::Openai(openai)) => {
							AIProvider::OpenAI(llm::openai::Provider {
								model: openai.model.as_deref().map(strng::new),
							})
						},
						Some(proto::agent::ai_backend::Provider::Gemini(gemini)) => {
							AIProvider::Gemini(llm::gemini::Provider {
								model: gemini.model.as_deref().map(strng::new),
							})
						},
						Some(proto::agent::ai_backend::Provider::Vertex(vertex)) => {
							AIProvider::Vertex(llm::vertex::Provider {
								model: vertex.model.as_deref().map(strng::new),
								region: Some(strng::new(&vertex.region)),
								project_id: strng::new(&vertex.project_id),
							})
						},
						Some(proto::agent::ai_backend::Provider::Anthropic(anthropic)) => {
							AIProvider::Anthropic(llm::anthropic::Provider {
								model: anthropic.model.as_deref().map(strng::new),
							})
						},
						Some(proto::agent::ai_backend::Provider::Bedrock(bedrock)) => {
							AIProvider::Bedrock(llm::bedrock::Provider {
								model: strng::new(
									bedrock
										.model
										.as_deref()
										.ok_or_else(|| ProtoError::Generic("bedrock requires a model".to_string()))?,
								),
								region: strng::new(&bedrock.region),
							})
						},
						None => {
							return Err(ProtoError::Generic(
								"AI backend provider is required".to_string(),
							));
						},
					},
				},
			),
			Some(proto::agent::backend::Kind::Mcp(m)) => Backend::MCP(
				name,
				McpBackend {
					targets: m
						.targets
						.iter()
						.map(|t| McpTarget::try_from(t).map(Arc::new))
						.collect::<Result<Vec<_>, _>>()?,
				},
			),
			_ => {
				return Err(ProtoError::Generic("unknown backend".to_string()));
			},
		})
	}
}

impl TryFrom<&proto::agent::McpTarget> for McpTarget {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::McpTarget) -> Result<Self, Self::Error> {
		let proto = proto::agent::mcp_target::Protocol::try_from(s.protocol)?;
		let backend = resolve_simple_reference(s.backend.as_ref())?;
		Ok(Self {
			name: strng::new(&s.name),
			spec: match proto {
				Protocol::Sse => McpTargetSpec::Sse(SseTargetSpec {
					backend,
					path: "/sse".to_string(),
				}),
				Protocol::Undefined | Protocol::StreamableHttp => {
					McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
						backend,
						path: "/mcp".to_string(),
					})
				},
			},
		})
	}
}

impl TryFrom<&proto::agent::RouteMatch> for RouteMatch {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::RouteMatch) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::path_match::*;
		let path = match &s.path {
			None => PathMatch::PathPrefix(strng::new("/")),
			Some(proto::agent::PathMatch {
				kind: Some(Kind::PathPrefix(prefix)),
			}) => PathMatch::PathPrefix(strng::new(prefix)),
			Some(proto::agent::PathMatch {
				kind: Some(Kind::Exact(prefix)),
			}) => PathMatch::Exact(strng::new(prefix)),
			Some(proto::agent::PathMatch {
				kind: Some(Kind::Regex(r)),
			}) => PathMatch::Regex(regex::Regex::new(r)?, r.len()),
			Some(proto::agent::PathMatch { kind: None }) => {
				return Err(ProtoError::Generic("invalid path match".to_string()));
			},
		};
		let method = s.method.as_ref().map(|m| MethodMatch {
			method: strng::new(&m.exact),
		});
		let headers = s
			.headers
			.iter()
			.map(|h| match &h.value {
				None => Err(ProtoError::Generic(
					"invalid header match value".to_string(),
				)),
				Some(proto::agent::header_match::Value::Exact(e)) => Ok(HeaderMatch {
					name: crate::http::HeaderName::from_bytes(h.name.as_bytes())?,
					value: HeaderValueMatch::Exact(crate::http::HeaderValue::from_bytes(e.as_bytes())?),
				}),
				Some(proto::agent::header_match::Value::Regex(e)) => Ok(HeaderMatch {
					name: crate::http::HeaderName::from_bytes(h.name.as_bytes())?,
					value: HeaderValueMatch::Regex(regex::Regex::new(e)?),
				}),
			})
			.collect::<Result<Vec<_>, _>>()?;
		let query = s
			.query_params
			.iter()
			.map(|h| match &h.value {
				None => Err(ProtoError::Generic("invalid query match value".to_string())),
				Some(proto::agent::query_match::Value::Exact(e)) => Ok(QueryMatch {
					name: strng::new(&h.name),
					value: QueryValueMatch::Exact(strng::new(e)),
				}),
				Some(proto::agent::query_match::Value::Regex(e)) => Ok(QueryMatch {
					name: strng::new(&h.name),
					value: QueryValueMatch::Regex(regex::Regex::new(e)?),
				}),
			})
			.collect::<Result<Vec<_>, _>>()?;
		Ok(Self {
			headers,
			path,
			method,
			query,
		})
	}
}

impl TryFrom<&proto::agent::RouteFilter> for RouteFilter {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::RouteFilter) -> Result<Self, Self::Error> {
		Ok(match &s.kind {
			None => return Err(ProtoError::Generic("invalid route filter".to_string())),
			Some(proto::agent::route_filter::Kind::RequestHeaderModifier(rhm)) => {
				RouteFilter::RequestHeaderModifier(filters::HeaderModifier {
					add: rhm
						.add
						.iter()
						.map(|h| (strng::new(&h.name), strng::new(&h.value)))
						.collect(),
					set: rhm
						.set
						.iter()
						.map(|h| (strng::new(&h.name), strng::new(&h.value)))
						.collect(),
					remove: rhm.remove.iter().map(strng::new).collect(),
				})
			},
			Some(proto::agent::route_filter::Kind::RequestRedirect(rd)) => {
				RouteFilter::RequestRedirect(filters::RequestRedirect {
					scheme: default_as_none(rd.scheme.as_str())
						.map(uri::Scheme::try_from)
						.transpose()?,
					authority: match (default_as_none(rd.host.as_str()), default_as_none(rd.port)) {
						(Some(h), Some(p)) => Some(HostRedirect::Full(strng::format!("{h}:{p}"))),
						(_, Some(p)) => Some(HostRedirect::Port(NonZeroU16::new(p as u16).unwrap())),
						(Some(h), _) => Some(HostRedirect::Host(strng::new(h))),
						(None, None) => None,
					},
					path: match &rd.path {
						Some(proto::agent::request_redirect::Path::Full(f)) => {
							Some(PathRedirect::Full(strng::new(f)))
						},
						Some(proto::agent::request_redirect::Path::Prefix(f)) => {
							Some(PathRedirect::Prefix(strng::new(f)))
						},
						None => None,
					},
					status: default_as_none(rd.status)
						.map(|i| StatusCode::from_u16(i as u16))
						.transpose()?,
				})
			},
			Some(proto::agent::route_filter::Kind::UrlRewrite(rw)) => {
				RouteFilter::UrlRewrite(filters::UrlRewrite {
					authority: default_as_none(rw.host.as_str()).map(|h| HostRedirect::Host(strng::new(h))),
					path: match &rw.path {
						Some(proto::agent::url_rewrite::Path::Full(f)) => {
							Some(PathRedirect::Full(strng::new(f)))
						},
						Some(proto::agent::url_rewrite::Path::Prefix(f)) => {
							Some(PathRedirect::Prefix(strng::new(f)))
						},
						None => None,
					},
				})
			},
			Some(proto::agent::route_filter::Kind::ResponseHeaderModifier(rhm)) => {
				RouteFilter::ResponseHeaderModifier(filters::HeaderModifier {
					add: rhm
						.add
						.iter()
						.map(|h| (strng::new(&h.name), strng::new(&h.value)))
						.collect(),
					set: rhm
						.set
						.iter()
						.map(|h| (strng::new(&h.name), strng::new(&h.value)))
						.collect(),
					remove: rhm.remove.iter().map(strng::new).collect(),
				})
			},
			Some(proto::agent::route_filter::Kind::RequestMirror(m)) => {
				let backend = resolve_simple_reference(m.backend.as_ref())?;
				RouteFilter::RequestMirror(filters::RequestMirror {
					backend,
					percentage: m.percentage / 100.0,
				})
			},
			Some(proto::agent::route_filter::Kind::DirectResponse(m)) => {
				RouteFilter::DirectResponse(filters::DirectResponse {
					body: Bytes::copy_from_slice(&m.body),
					status: StatusCode::from_u16(m.status as u16)?,
				})
			},
			Some(proto::agent::route_filter::Kind::Cors(c)) => RouteFilter::CORS(
				http::cors::Cors::try_from(http::cors::CorsSerde {
					allow_credentials: c.allow_credentials,
					allow_headers: c.allow_headers.clone(),
					allow_methods: c.allow_methods.clone(),
					allow_origins: c.allow_origins.clone(),
					expose_headers: c.expose_headers.clone(),
					max_age: c.max_age.map(|d| Duration::from_secs(d.seconds as u64)),
				})
				.map_err(|e| ProtoError::Generic(e.to_string()))?,
			),
		})
	}
}

fn default_as_none<T: Default + PartialEq>(i: T) -> Option<T> {
	if i == Default::default() {
		None
	} else {
		Some(i)
	}
}

impl TryFrom<&proto::agent::Policy> for TargetedPolicy {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Policy) -> Result<Self, Self::Error> {
		let name = PolicyName::from(&s.name);
		let target = s.target.as_ref().ok_or(ProtoError::MissingRequiredField)?;
		let spec = s.spec.as_ref().ok_or(ProtoError::MissingRequiredField)?;
		let target = match &target.kind {
			Some(proto::agent::policy_target::Kind::Gateway(v)) => PolicyTarget::Gateway(v.into()),
			Some(proto::agent::policy_target::Kind::Listener(v)) => PolicyTarget::Listener(v.into()),
			Some(proto::agent::policy_target::Kind::Route(v)) => PolicyTarget::Route(v.into()),
			Some(proto::agent::policy_target::Kind::RouteRule(v)) => PolicyTarget::RouteRule(v.into()),
			Some(proto::agent::policy_target::Kind::Backend(v)) => PolicyTarget::Backend(v.into()),
			_ => return Err(ProtoError::EnumParse("unknown target kind".to_string())),
		};
		let policy = match &spec.kind {
			Some(proto::agent::policy_spec::Kind::LocalRateLimit(lrl)) => {
				let t = proto::agent::policy_spec::local_rate_limit::Type::try_from(lrl.r#type)?;
				Policy::LocalRateLimit(vec![
					localratelimit::RateLimitSerde {
						max_tokens: lrl.max_tokens,
						tokens_per_fill: lrl.tokens_per_fill,
						fill_interval: lrl
							.fill_interval
							.ok_or(ProtoError::MissingRequiredField)?
							.try_into()?,
						limit_type: match t {
							Type::Request => localratelimit::RateLimitType::Requests,
							Type::Token => localratelimit::RateLimitType::Tokens,
						},
					}
					.try_into()
					.map_err(|e| ProtoError::Generic(format!("invalid rate limit: {e}")))?,
				])
			},
			Some(proto::agent::policy_spec::Kind::ExtAuthz(ea)) => {
				let target = resolve_simple_reference(ea.target.as_ref())?;
				Policy::ExtAuthz(http::ext_authz::ExtAuthz {
					target: Arc::new(target),
					context: Some(ea.context.clone()),
				})
			},
			Some(proto::agent::policy_spec::Kind::A2a(a2a)) => Policy::A2a(A2aPolicy {}),
			Some(proto::agent::policy_spec::Kind::InferenceRouting(ir)) => {
				Policy::InferenceRouting(ext_proc::InferenceRouting {
					target: Arc::new(resolve_simple_reference(ir.endpoint_picker.as_ref())?),
				})
			},
			_ => return Err(ProtoError::EnumParse("unknown spec kind".to_string())),
		};
		Ok(TargetedPolicy {
			name,
			target,
			policy,
		})
	}
}

fn resolve_simple_reference(
	target: Option<&proto::agent::BackendReference>,
) -> Result<SimpleBackendReference, ProtoError> {
	let Some(target) = target else {
		return Ok(SimpleBackendReference::Invalid);
	};
	Ok(match target.kind.as_ref() {
		None => SimpleBackendReference::Invalid,
		Some(proto::agent::backend_reference::Kind::Service(svc_key)) => {
			let ns = match svc_key.split_once('/') {
				Some((namespace, hostname)) => Ok(NamespacedHostname {
					namespace: namespace.into(),
					hostname: hostname.into(),
				}),
				None => Err(ProtoError::NamespacedHostnameParse(svc_key.clone())),
			}?;
			SimpleBackendReference::Service {
				name: ns,
				port: target.port as u16,
			}
		},
		Some(proto::agent::backend_reference::Kind::Backend(name)) => {
			SimpleBackendReference::Backend(name.into())
		},
	})
}

fn resolve_reference(
	target: Option<&proto::agent::BackendReference>,
) -> Result<BackendReference, ProtoError> {
	let Some(target) = target else {
		return Ok(BackendReference::Invalid);
	};
	Ok(match target.kind.as_ref() {
		None => BackendReference::Invalid,
		Some(proto::agent::backend_reference::Kind::Service(svc_key)) => {
			let ns = match svc_key.split_once('/') {
				Some((namespace, hostname)) => Ok(NamespacedHostname {
					namespace: namespace.into(),
					hostname: hostname.into(),
				}),
				None => Err(ProtoError::NamespacedHostnameParse(svc_key.clone())),
			}?;
			BackendReference::Service {
				name: ns,
				port: target.port as u16,
			}
		},
		Some(proto::agent::backend_reference::Kind::Backend(name)) => {
			BackendReference::Backend(name.into())
		},
	})
}
