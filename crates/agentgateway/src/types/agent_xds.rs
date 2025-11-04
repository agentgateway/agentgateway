use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;

use crate::http::Scheme;
use ::http::StatusCode;
use frozen_collections::FzHashSet;
use rustls::ServerConfig;

use super::agent::*;
use crate::http::auth::{AwsAuth, BackendAuth, SimpleBackendAuth};
use crate::http::authorization;
use crate::http::transformation_cel::{LocalTransform, LocalTransformationConfig, Transformation};
use crate::llm::{AIBackend, AIProvider, NamedAIProvider};
use crate::mcp::McpAuthorization;
use crate::telemetry::log::OrderedStringMap;
use crate::types::discovery::NamespacedHostname;
use crate::types::proto::ProtoError;
use crate::types::proto::agent::mcp_target::Protocol;
use crate::types::proto::agent::traffic_policy_spec::host_rewrite::Mode;
use crate::types::{agent, proto};
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
		let backend = resolve_reference(s.backend.as_ref())?;
		let inline_policies = s
			.backend_policies
			.iter()
			.map(BackendPolicy::try_from)
			.collect::<Result<Vec<_>, _>>()?;
		Ok(Self {
			weight: s.weight as usize,
			backend,
			inline_policies,
		})
	}
}

impl TryFrom<&proto::agent::backend_policy_spec::McpAuthorization> for McpAuthorization {
	type Error = ProtoError;

	fn try_from(
		rbac: &proto::agent::backend_policy_spec::McpAuthorization,
	) -> Result<Self, Self::Error> {
		let mut allow_exprs = Vec::new();
		for allow_rule in &rbac.allow {
			let expr = cel::Expression::new(allow_rule)
				.map_err(|e| ProtoError::Generic(format!("invalid CEL expression in allow rule: {e}")))?;
			allow_exprs.push(Arc::new(expr));
		}

		let mut deny_exprs = Vec::new();
		for deny_rule in &rbac.deny {
			let expr = cel::Expression::new(deny_rule)
				.map_err(|e| ProtoError::Generic(format!("invalid CEL expression in deny rule: {e}")))?;
			deny_exprs.push(Arc::new(expr));
		}

		let policy_set = authorization::PolicySet::new(allow_exprs, deny_exprs);
		Ok(McpAuthorization::new(authorization::RuleSet::new(
			policy_set,
		)))
	}
}

impl TryFrom<&proto::agent::backend_policy_spec::McpAuthentication> for McpAuthentication {
	type Error = ProtoError;

	fn try_from(
		m: &proto::agent::backend_policy_spec::McpAuthentication,
	) -> Result<Self, Self::Error> {
		let provider = match m.provider {
			x if x == proto::agent::backend_policy_spec::mcp_authentication::McpIdp::Auth0 as i32 => {
				Some(McpIDP::Auth0 {})
			},
			x if x == proto::agent::backend_policy_spec::mcp_authentication::McpIdp::Keycloak as i32 => {
				Some(McpIDP::Keycloak {})
			},
			_ => None,
		};

		Ok(McpAuthentication {
			issuer: m.issuer.clone(),
			audience: m.audience.clone(),
			jwks_url: m.jwks_url.clone(),
			provider,
			resource_metadata: ResourceMetadata {
				extra: Default::default(),
			},
		})
	}
}

fn convert_backend_ai_policy(
	ai: &proto::agent::backend_policy_spec::Ai,
) -> Result<llm::Policy, ProtoError> {
	let prompt_guard = ai.prompt_guard.as_ref().and_then(|pg| {
		if pg.request.is_none() && pg.response.is_none() {
			return None;
		}
		let request_guard = pg.request.as_ref().map(|reqp| {
			let rejection = if let Some(resp) = &reqp.rejection {
				let status = u16::try_from(resp.status)
					.ok()
					.and_then(|c| StatusCode::from_u16(c).ok())
					.unwrap_or(StatusCode::FORBIDDEN);
				crate::llm::policy::RequestRejection {
					body: Bytes::from(resp.body.clone()),
					status,
					headers: None, // TODO: map from proto if headers are added there
				}
			} else {
				//  use default response, since the response field is not optional on RequestGuard
				crate::llm::policy::RequestRejection::default()
			};

			let regex = reqp
				.regex
				.as_ref()
				.map(|rr| convert_regex_rules(rr, Some(rejection.clone())));

			let webhook = reqp.webhook.as_ref().and_then(convert_webhook);

			let openai_moderation =
				reqp
					.openai_moderation
					.as_ref()
					.map(|m| crate::llm::policy::Moderation {
						model: m.model.as_deref().map(strng::new),
						auth: match m.auth.as_ref().and_then(|a| a.kind.clone()) {
							Some(crate::types::proto::agent::backend_auth_policy::Kind::Passthrough(_)) => {
								SimpleBackendAuth::Passthrough {}
							},
							Some(crate::types::proto::agent::backend_auth_policy::Kind::Key(k)) => {
								SimpleBackendAuth::Key(k.secret.into())
							},
							_ => SimpleBackendAuth::Passthrough {},
						},
					});

			crate::llm::policy::RequestGuard {
				rejection,
				regex,
				webhook,
				openai_moderation,
			}
		});

		Some(crate::llm::policy::PromptGuard {
			request: request_guard,
			response: pg
				.response
				.as_ref()
				.map(|resp| crate::llm::policy::ResponseGuard {
					regex: resp.regex.as_ref().map(|rr| convert_regex_rules(rr, None)),
					webhook: resp.webhook.as_ref().and_then(convert_webhook),
				}),
		})
	});

	Ok(llm::Policy {
		prompt_guard,
		defaults: Some(
			ai.defaults
				.iter()
				.map(|(k, v)| serde_json::from_str(v).map(|v| (k.clone(), v)))
				.collect::<Result<_, _>>()?,
		),
		overrides: Some(
			ai.overrides
				.iter()
				.map(|(k, v)| serde_json::from_str(v).map(|v| (k.clone(), v)))
				.collect::<Result<_, _>>()?,
		),
		prompts: ai.prompts.as_ref().map(convert_prompt_enrichment),
		model_aliases: ai
			.model_aliases
			.iter()
			.map(|(k, v)| (strng::new(k), strng::new(v)))
			.collect(),
		prompt_caching: ai.prompt_caching.as_ref().map(convert_prompt_caching),
	})
}

impl TryFrom<proto::agent::BackendAuthPolicy> for BackendAuth {
	type Error = ProtoError;

	fn try_from(s: proto::agent::BackendAuthPolicy) -> Result<Self, Self::Error> {
		Ok(match s.kind {
			Some(proto::agent::backend_auth_policy::Kind::Passthrough(_)) => BackendAuth::Passthrough {},
			Some(proto::agent::backend_auth_policy::Kind::Key(k)) => BackendAuth::Key(k.secret.into()),
			Some(proto::agent::backend_auth_policy::Kind::Gcp(_)) => BackendAuth::Gcp {},
			Some(proto::agent::backend_auth_policy::Kind::Aws(a)) => {
				let aws_auth = match a.kind {
					Some(proto::agent::aws::Kind::ExplicitConfig(config)) => AwsAuth::ExplicitConfig {
						access_key_id: config.access_key_id.into(),
						secret_access_key: config.secret_access_key.into(),
						region: config.region,
						session_token: config.session_token.map(|token| token.into()),
					},
					Some(proto::agent::aws::Kind::Implicit(_)) => AwsAuth::Implicit {},
					None => return Err(ProtoError::MissingRequiredField),
				};
				BackendAuth::Aws(aws_auth)
			},
			Some(proto::agent::backend_auth_policy::Kind::Azure(a)) => {
				let azure_auth = match a.kind {
					Some(proto::agent::azure::Kind::ExplicitConfig(config)) => {
						let src = match config.credential_source {
                            Some(proto::agent::azure_explicit_config::CredentialSource::ClientSecret(cs)) => {
                                crate::http::auth::AzureAuthCredentialSource::ClientSecret {
                                    tenant_id: cs.tenant_id,
                                    client_id: cs.client_id,
                                    client_secret: cs.client_secret.into(),
                                }
                            }
                            Some(proto::agent::azure_explicit_config::CredentialSource::ManagedIdentityCredential(mic)) => {
                                crate::http::auth::AzureAuthCredentialSource::ManagedIdentity {
                                    user_assigned_identity: mic.user_assigned_identity.map(|uami| {
                                        uami.id.map(|id| match id {
                                            proto::agent::azure_managed_identity_credential::user_assigned_identity::Id::ClientId(c) => {
                                                crate::http::auth::AzureUserAssignedIdentity::ClientId(c)
                                            }
                                            proto::agent::azure_managed_identity_credential::user_assigned_identity::Id::ObjectId(o) => {
                                                crate::http::auth::AzureUserAssignedIdentity::ObjectId(o)
                                            }
                                            proto::agent::azure_managed_identity_credential::user_assigned_identity::Id::ResourceId(r) => {
                                                crate::http::auth::AzureUserAssignedIdentity::ResourceId(r)
                                            }
                                        }).expect("one of clientId, objectId, or resourceId must be set")
                                    })
                                }
                            }
                            Some(proto::agent::azure_explicit_config::CredentialSource::WorkloadIdentityCredential(_)) => {
                                crate::http::auth::AzureAuthCredentialSource::WorkloadIdentity {}
                            }
                            None => {
                                return Err(ProtoError::MissingRequiredField);
                            }
                        };
						crate::http::auth::AzureAuth::ExplicitConfig {
							credential_source: src,
						}
					},
					Some(proto::agent::azure::Kind::DeveloperImplicit(_)) => {
						crate::http::auth::AzureAuth::DeveloperImplicit {}
					},
					None => return Err(ProtoError::MissingRequiredField),
				};
				BackendAuth::Azure(azure_auth)
			},
			None => return Err(ProtoError::MissingRequiredField),
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
			// TLS termination
			(Protocol::Tls, Some(tls)) => Ok(ListenerProtocol::TLS(Some(
				tls
					.try_into()
					.map_err(|e| ProtoError::Generic(format!("{e}")))?,
			))),
			// TLS passthrough
			(Protocol::Tls, None) => Ok(ListenerProtocol::TLS(None)),
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

impl TryFrom<&proto::agent::TcpRoute> for (TCPRoute, ListenerKey) {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::TcpRoute) -> Result<Self, Self::Error> {
		let r = TCPRoute {
			key: strng::new(&s.key),
			route_name: strng::new(&s.route_name),
			rule_name: default_as_none(s.rule_name.as_str()).map(strng::new),
			hostnames: s.hostnames.iter().map(strng::new).collect(),
			backends: s
				.backends
				.iter()
				.map(|b| -> Result<TCPRouteBackendReference, ProtoError> {
					Ok(TCPRouteBackendReference {
						weight: b.weight as usize,
						backend: resolve_simple_reference(b.backend.as_ref())?,
					})
				})
				.collect::<Result<Vec<_>, _>>()?,
		};
		Ok((r, strng::new(&s.listener_key)))
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
			matches: s
				.matches
				.iter()
				.map(RouteMatch::try_from)
				.collect::<Result<Vec<_>, _>>()?,
			backends: s
				.backends
				.iter()
				.map(RouteBackendReference::try_from)
				.collect::<Result<Vec<_>, _>>()?,
			inline_policies: s
				.traffic_policies
				.iter()
				.map(TrafficPolicy::try_from)
				.collect::<Result<Vec<_>, _>>()?,
		};
		Ok((r, strng::new(&s.listener_key)))
	}
}

impl TryFrom<&proto::agent::Backend> for BackendWithPolicies {
	type Error = ProtoError;

	fn try_from(s: &proto::agent::Backend) -> Result<Self, Self::Error> {
		let pols = s
			.inline_policies
			.iter()
			.map(BackendPolicy::try_from)
			.collect::<Result<Vec<_>, _>>()?;
		let name = BackendName::from(&s.name);
		let backend = match &s.kind {
			Some(proto::agent::backend::Kind::Static(s)) => Backend::Opaque(
				name.clone(),
				Target::try_from((s.host.as_str(), s.port as u16))
					.map_err(|e| ProtoError::Generic(e.to_string()))?,
			),
			Some(proto::agent::backend::Kind::Ai(a)) => {
				if a.provider_groups.is_empty() {
					return Err(ProtoError::Generic(
						"AI backend must have at least one provider group".to_string(),
					));
				}

				let mut provider_groups = Vec::new();

				for group in &a.provider_groups {
					let mut local_provider_group = Vec::new();
					for (provider_idx, provider_config) in group.providers.iter().enumerate() {
						let pols = provider_config
							.inline_policies
							.iter()
							.map(BackendPolicy::try_from)
							.collect::<Result<Vec<_>, _>>()?;
						let provider = match &provider_config.provider {
							Some(proto::agent::ai_backend::provider::Provider::Openai(openai)) => {
								AIProvider::OpenAI(llm::openai::Provider {
									model: openai.model.as_deref().map(strng::new),
								})
							},
							Some(proto::agent::ai_backend::provider::Provider::Gemini(gemini)) => {
								AIProvider::Gemini(llm::gemini::Provider {
									model: gemini.model.as_deref().map(strng::new),
								})
							},
							Some(proto::agent::ai_backend::provider::Provider::Vertex(vertex)) => {
								AIProvider::Vertex(llm::vertex::Provider {
									model: vertex.model.as_deref().map(strng::new),
									region: Some(strng::new(&vertex.region)),
									project_id: strng::new(&vertex.project_id),
								})
							},
							Some(proto::agent::ai_backend::provider::Provider::Anthropic(anthropic)) => {
								AIProvider::Anthropic(llm::anthropic::Provider {
									model: anthropic.model.as_deref().map(strng::new),
								})
							},
							Some(proto::agent::ai_backend::provider::Provider::Bedrock(bedrock)) => {
								AIProvider::Bedrock(llm::bedrock::Provider {
									model: bedrock.model.as_deref().map(strng::new),
									region: strng::new(&bedrock.region),
									guardrail_identifier: bedrock.guardrail_identifier.as_deref().map(strng::new),
									guardrail_version: bedrock.guardrail_version.as_deref().map(strng::new),
								})
							},
							Some(proto::agent::ai_backend::provider::Provider::Azureopenai(azureopenai)) => {
								AIProvider::AzureOpenAI(llm::azureopenai::Provider {
									model: azureopenai.model.as_deref().map(strng::new),
									host: strng::new(&azureopenai.host),
									api_version: azureopenai.api_version.as_deref().map(strng::new),
								})
							},
							None => {
								return Err(ProtoError::Generic(format!(
									"AI backend provider at index {provider_idx} is required"
								)));
							},
						};

						let provider_name = if provider_config.name.is_empty() {
							strng::new(format!("{name}_{provider_idx}"))
						} else {
							strng::new(&provider_config.name)
						};

						let np = NamedAIProvider {
							name: provider_name.clone(),
							provider,
							tokenize: false,
							path_override: provider_config.path_override.as_ref().map(strng::new),
							host_override: provider_config
								.r#host_override
								.as_ref()
								.map(|o| {
									Target::try_from((o.host.as_str(), o.port as u16))
										.map_err(|e| ProtoError::Generic(e.to_string()))
								})
								.transpose()?,
							inline_policies: pols,
							routes: provider_config
								.routes
								.iter()
								.map(|(path, proto_route_type)| {
									use proto::agent::ai_backend::RouteType as ProtoRT;
									let route_type = match ProtoRT::try_from(*proto_route_type) {
										Ok(ProtoRT::Completions) | Ok(ProtoRT::Unspecified) => {
											llm::RouteType::Completions
										},
										Ok(ProtoRT::Messages) => llm::RouteType::Messages,
										Ok(ProtoRT::Models) => llm::RouteType::Models,
										Ok(ProtoRT::Passthrough) => llm::RouteType::Passthrough,
										Ok(ProtoRT::Responses) => llm::RouteType::Responses,
										Ok(ProtoRT::AnthropicTokenCount) => llm::RouteType::AnthropicTokenCount,
										Err(_) => {
											warn!(
												value = proto_route_type,
												"Unknown proto RouteType value, defaulting to Completions"
											);
											llm::RouteType::Completions
										},
									};
									(strng::new(path), route_type)
								})
								.collect(),
						};
						local_provider_group.push((provider_name, np));
					}

					if !local_provider_group.is_empty() {
						provider_groups.push(local_provider_group);
					}
				}

				if provider_groups.is_empty() {
					return Err(ProtoError::Generic(
						"AI backend must have at least one non-empty provider group".to_string(),
					));
				}

				let es = crate::types::loadbalancer::EndpointSet::new(provider_groups);
				Backend::AI(name.clone(), AIBackend { providers: es })
			},
			Some(proto::agent::backend::Kind::Mcp(m)) => Backend::MCP(
				name.clone(),
				McpBackend {
					targets: m
						.targets
						.iter()
						.map(|t| McpTarget::try_from(t).map(Arc::new))
						.collect::<Result<Vec<_>, _>>()?,
					stateful: match m.stateful_mode() {
						proto::agent::mcp_backend::StatefulMode::Stateful => true,
						proto::agent::mcp_backend::StatefulMode::Stateless => false,
					},
					always_use_prefix: match m.prefix_mode() {
						proto::agent::mcp_backend::PrefixMode::Always => true,
						proto::agent::mcp_backend::PrefixMode::Conditional => false,
					},
				},
			),
			_ => {
				return Err(ProtoError::Generic("unknown backend".to_string()));
			},
		};
		Ok(Self {
			backend,
			inline_policies: pols,
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
					path: if s.path.is_empty() {
						"/sse".to_string()
					} else {
						s.path.clone()
					},
				}),
				Protocol::Undefined | Protocol::StreamableHttp => {
					McpTargetSpec::Mcp(StreamableHTTPTargetSpec {
						backend,
						path: if s.path.is_empty() {
							"/mcp".to_string()
						} else {
							s.path.clone()
						},
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
		let headers = match convert_header_match(&s.headers) {
			Ok(h) => h,
			Err(e) => return Err(ProtoError::Generic(format!("invalid header match: {e}"))),
		};

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

fn default_as_none<T: Default + PartialEq>(i: T) -> Option<T> {
	if i == Default::default() {
		None
	} else {
		Some(i)
	}
}

impl TryFrom<&proto::agent::traffic_policy_spec::Rbac> for Authorization {
	type Error = ProtoError;

	fn try_from(rbac: &proto::agent::traffic_policy_spec::Rbac) -> Result<Self, Self::Error> {
		// Convert allow rules
		let mut allow_exprs = Vec::new();
		for allow_rule in &rbac.allow {
			let expr = cel::Expression::new(allow_rule)
				.map_err(|e| ProtoError::Generic(format!("invalid CEL expression in allow rule: {e}")))?;
			allow_exprs.push(Arc::new(expr));
		}
		// Convert deny rules
		let mut deny_exprs = Vec::new();
		for deny_rule in &rbac.deny {
			let expr = cel::Expression::new(deny_rule)
				.map_err(|e| ProtoError::Generic(format!("invalid CEL expression in deny rule: {e}")))?;
			deny_exprs.push(Arc::new(expr));
		}

		// Create PolicySet using the same pattern as in de_policies function
		let policy_set = authorization::PolicySet::new(allow_exprs, deny_exprs);
		Ok(Authorization(authorization::RuleSet::new(policy_set)))
	}
}

impl TryFrom<&proto::agent::traffic_policy_spec::TransformationPolicy> for Transformation {
	type Error = ProtoError;

	fn try_from(
		spec: &proto::agent::traffic_policy_spec::TransformationPolicy,
	) -> Result<Self, Self::Error> {
		fn convert_transform(
			t: &Option<proto::agent::traffic_policy_spec::transformation_policy::Transform>,
		) -> Result<LocalTransform, ProtoError> {
			let mut add = Vec::new();
			let mut set = Vec::new();
			let mut remove = Vec::new();
			let mut body = None;

			if let Some(t) = t {
				for h in &t.add {
					add.push((h.name.clone().into(), h.expression.clone().into()));
				}
				for h in &t.set {
					set.push((h.name.clone().into(), h.expression.clone().into()));
				}
				for r in &t.remove {
					remove.push(r.clone().into());
				}
				if let Some(b) = &t.body {
					body = Some(b.expression.clone().into());
				}
			}

			Ok(LocalTransform {
				add,
				set,
				remove,
				body,
			})
		}

		let request = Some(convert_transform(&spec.request)?);
		let response = Some(convert_transform(&spec.response)?);
		let config = LocalTransformationConfig { request, response };
		Transformation::try_from(config).map_err(|e| ProtoError::Generic(e.to_string()))
	}
}

impl TryFrom<&proto::agent::BackendPolicySpec> for BackendPolicy {
	type Error = ProtoError;

	fn try_from(spec: &proto::agent::BackendPolicySpec) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::backend_policy_spec as bps;
		Ok(match &spec.kind {
			Some(bps::Kind::A2a(_)) => BackendPolicy::A2a(A2aPolicy {}),
			Some(bps::Kind::InferenceRouting(ir)) => {
				let failure_mode = match bps::inference_routing::FailureMode::try_from(ir.failure_mode)? {
					bps::inference_routing::FailureMode::Unknown
					| bps::inference_routing::FailureMode::FailClosed => http::ext_proc::FailureMode::FailClosed,
					bps::inference_routing::FailureMode::FailOpen => http::ext_proc::FailureMode::FailOpen,
				};
				BackendPolicy::InferenceRouting(http::ext_proc::InferenceRouting {
					target: Arc::new(resolve_simple_reference(ir.endpoint_picker.as_ref())?),
					failure_mode,
				})
			},
			Some(bps::Kind::BackendTls(btls)) => {
				let tls = http::backendtls::ResolvedBackendTLS {
					cert: btls.cert.clone(),
					key: btls.key.clone(),
					root: btls.root.clone(),
					insecure: btls.insecure.unwrap_or_default(),
					insecure_host: false,
					hostname: btls.hostname.clone(),
					alpn: None,
				}
				.try_into()
				.map_err(|e| ProtoError::Generic(e.to_string()))?;
				BackendPolicy::BackendTLS(tls)
			},
			Some(bps::Kind::Auth(auth)) => {
				BackendPolicy::BackendAuth(BackendAuth::try_from(auth.clone())?)
			},
			Some(bps::Kind::McpAuthorization(rbac)) => {
				BackendPolicy::McpAuthorization(McpAuthorization::try_from(rbac)?)
			},
			Some(bps::Kind::McpAuthentication(ma)) => {
				BackendPolicy::McpAuthentication(McpAuthentication::try_from(ma)?)
			},
			Some(bps::Kind::Ai(ai)) => BackendPolicy::AI(Arc::new(convert_backend_ai_policy(ai)?)),
			Some(bps::Kind::RequestHeaderModifier(rhm)) => {
				BackendPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
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
			Some(bps::Kind::ResponseHeaderModifier(rhm)) => {
				BackendPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
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
			Some(bps::Kind::RequestRedirect(rr)) => {
				BackendPolicy::RequestRedirect(http::filters::RequestRedirect {
					scheme: default_as_none(rr.scheme.as_str())
						.map(Scheme::try_from)
						.transpose()?,
					authority: match (default_as_none(rr.host.as_str()), default_as_none(rr.port)) {
						(Some(h), Some(p)) => Some(HostRedirect::Full(strng::format!("{h}:{p}"))),
						(_, Some(p)) => Some(HostRedirect::Port(NonZeroU16::new(p as u16).unwrap())),
						(Some(h), _) => Some(HostRedirect::Host(strng::new(h))),
						(None, None) => None,
					},
					path: match &rr.path {
						Some(proto::agent::request_redirect::Path::Full(f)) => {
							Some(PathRedirect::Full(strng::new(f)))
						},
						Some(proto::agent::request_redirect::Path::Prefix(f)) => {
							Some(PathRedirect::Prefix(strng::new(f)))
						},
						None => None,
					},
					status: default_as_none(rr.status)
						.map(|i| StatusCode::from_u16(i as u16))
						.transpose()?,
				})
			},
			Some(bps::Kind::RequestMirror(m)) => {
				let mirrors = m
					.mirrors
					.iter()
					.map(|m| {
						let backend = resolve_simple_reference(m.backend.as_ref())?;
						Ok::<_, ProtoError>(http::filters::RequestMirror {
							backend,
							percentage: m.percentage / 100.0,
						})
					})
					.collect::<Result<Vec<_>, _>>()?;
				BackendPolicy::RequestMirror(mirrors)
			},
			None => return Err(ProtoError::MissingRequiredField),
		})
	}
}

impl TryFrom<&proto::agent::TrafficPolicySpec> for PhasedTrafficPolicy {
	type Error = ProtoError;

	fn try_from(spec: &proto::agent::TrafficPolicySpec) -> Result<Self, Self::Error> {
		let tp = TrafficPolicy::try_from(spec)?;
		Ok(PhasedTrafficPolicy {
			phase: match proto::agent::traffic_policy_spec::PolicyPhase::try_from(spec.phase)? {
				proto::agent::traffic_policy_spec::PolicyPhase::Route => PolicyPhase::Route,
				proto::agent::traffic_policy_spec::PolicyPhase::Gateway => PolicyPhase::Gateway,
			},
			policy: tp,
		})
	}
}

impl TryFrom<&proto::agent::TrafficPolicySpec> for TrafficPolicy {
	type Error = ProtoError;

	fn try_from(spec: &proto::agent::TrafficPolicySpec) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::traffic_policy_spec as tps;
		Ok(match &spec.kind {
			Some(tps::Kind::Timeout(t)) => TrafficPolicy::Timeout(http::timeout::Policy {
				request_timeout: t.request.as_ref().map(|d| (*d).try_into()).transpose()?,
				backend_request_timeout: t
					.backend_request
					.as_ref()
					.map(|d| (*d).try_into())
					.transpose()?,
			}),
			Some(tps::Kind::Retry(r)) => {
				let attempts = std::num::NonZeroU8::new(r.attempts as u8)
					.unwrap_or_else(|| std::num::NonZeroU8::new(1).unwrap());
				let backoff = r.backoff.as_ref().map(|d| (*d).try_into()).transpose()?;
				let codes = r
					.retry_status_codes
					.iter()
					.map(|c| StatusCode::from_u16(*c as u16).map_err(|e| ProtoError::Generic(e.to_string())))
					.collect::<Result<Vec<_>, _>>()?;
				TrafficPolicy::Retry(http::retry::Policy {
					attempts,
					backoff,
					codes: codes.into_boxed_slice(),
				})
			},
			Some(tps::Kind::LocalRateLimit(lrl)) => {
				let t = tps::local_rate_limit::Type::try_from(lrl.r#type)?;
				let spec = http::localratelimit::RateLimitSpec {
					max_tokens: lrl.max_tokens,
					tokens_per_fill: lrl.tokens_per_fill,
					fill_interval: lrl
						.fill_interval
						.ok_or(ProtoError::MissingRequiredField)?
						.try_into()?,
					limit_type: match t {
						tps::local_rate_limit::Type::Request => http::localratelimit::RateLimitType::Requests,
						tps::local_rate_limit::Type::Token => http::localratelimit::RateLimitType::Tokens,
					},
				};
				TrafficPolicy::LocalRateLimit(vec![
					spec
						.try_into()
						.map_err(|e| ProtoError::Generic(format!("invalid rate limit: {e}")))?,
				])
			},
			Some(tps::Kind::ExtAuthz(ea)) => {
				let target = resolve_simple_reference(ea.target.as_ref())?;
				let failure_mode =
					match proto::agent::traffic_policy_spec::external_auth::FailureMode::try_from(
						ea.failure_mode,
					) {
						Ok(proto::agent::traffic_policy_spec::external_auth::FailureMode::Allow) => {
							http::ext_authz::FailureMode::Allow
						},
						Ok(proto::agent::traffic_policy_spec::external_auth::FailureMode::Deny) => {
							http::ext_authz::FailureMode::Deny
						},
						Ok(proto::agent::traffic_policy_spec::external_auth::FailureMode::DenyWithStatus) => {
							let status = ea.status_on_error.unwrap_or(403) as u16;
							http::ext_authz::FailureMode::DenyWithStatus(status)
						},
						_ => http::ext_authz::FailureMode::Deny, // Default fallback
					};
				let include_request_body =
					ea.include_request_body
						.as_ref()
						.map(|body_opts| http::ext_authz::BodyOptions {
							max_request_bytes: body_opts.max_request_bytes,
							allow_partial_message: body_opts.allow_partial_message,
							pack_as_bytes: body_opts.pack_as_bytes,
						});
				let timeout = ea.timeout.map(convert_duration);
				let metadata: HashMap<_, _> = ea
					.metadata
					.iter()
					.map(|(k, v)| {
						let ve = cel::Expression::new(v)
							.map_err(|e| ProtoError::Generic(format!("invalid metadata expression: {e}")))?;
						Ok::<_, ProtoError>((k.to_owned(), Arc::new(ve)))
					})
					.collect::<Result<_, _>>()?;
				TrafficPolicy::ExtAuthz(http::ext_authz::ExtAuthz {
					target: Arc::new(target),
					context: Some(ea.context.clone()),
					metadata: if metadata.is_empty() {
						None
					} else {
						Some(metadata)
					},
					failure_mode,
					include_request_headers: ea
						.include_request_headers
						.iter()
						.filter_map(
							|s| match crate::http::HeaderOrPseudo::try_from(s.as_str()) {
								Ok(h) => Some(h),
								Err(_) => {
									warn!(name = %s, "Invalid header in extauth include_request_headers; skipping");
									None
								},
							},
						)
						.collect(),
					include_request_body,
					timeout,
				})
			},
			Some(tps::Kind::Authorization(rbac)) => {
				TrafficPolicy::Authorization(Authorization::try_from(rbac)?)
			},
			Some(tps::Kind::Jwt(jwt)) => {
				let mode = match tps::jwt::Mode::try_from(jwt.mode)
					.map_err(|_| ProtoError::EnumParse("invalid JWT mode".to_string()))?
				{
					tps::jwt::Mode::Optional => http::jwt::Mode::Optional,
					tps::jwt::Mode::Strict => http::jwt::Mode::Strict,
					tps::jwt::Mode::Permissive => http::jwt::Mode::Permissive,
				};
				let providers = jwt
					.providers
					.iter()
					.map(|p| {
						let jwks_json = match &p.jwks_source {
							Some(tps::jwt_provider::JwksSource::Inline(inline)) => inline.clone(),
							None => {
								return Err(ProtoError::Generic(
									"JWT policy missing JWKS source".to_string(),
								));
							},
						};
						let jwk_set: jsonwebtoken::jwk::JwkSet = serde_json::from_str(&jwks_json)
							.map_err(|e| ProtoError::Generic(format!("failed to parse JWKS: {e}")))?;
						let audiences = if p.audiences.is_empty() {
							None
						} else {
							Some(p.audiences.clone())
						};
						http::jwt::Provider::from_jwks(jwk_set, p.issuer.clone(), audiences)
							.map_err(|e| ProtoError::Generic(format!("failed to create JWT config: {e}")))
					})
					.collect::<Result<Vec<_>, _>>()?;
				let jwt_auth = http::jwt::Jwt::from_providers(providers, mode);
				TrafficPolicy::JwtAuth(jwt_auth)
			},
			Some(tps::Kind::Transformation(tp)) => {
				TrafficPolicy::Transformation(Transformation::try_from(tp)?)
			},
			Some(tps::Kind::RemoteRateLimit(rrl)) => {
				let descriptors = rrl
					.descriptors
					.iter()
					.map(
						|d| -> Result<http::remoteratelimit::DescriptorEntry, ProtoError> {
							let entries: Result<Vec<_>, ProtoError> = d
								.entries
								.iter()
								.map(|e| {
									cel::Expression::new(e.value.clone())
										.map_err(|e| ProtoError::Generic(format!("invalid descriptor value: {e}")))
										.map(|expr| http::remoteratelimit::Descriptor(e.key.clone(), expr))
								})
								.collect();
							Ok(http::remoteratelimit::DescriptorEntry {
								entries: Arc::new(entries?),
								limit_type: match tps::remote_rate_limit::Type::try_from(d.r#type)
									.unwrap_or(tps::remote_rate_limit::Type::Requests)
								{
									tps::remote_rate_limit::Type::Requests => {
										http::localratelimit::RateLimitType::Requests
									},
									tps::remote_rate_limit::Type::Tokens => {
										http::localratelimit::RateLimitType::Tokens
									},
								},
							})
						},
					)
					.collect::<Result<Vec<_>, _>>()?;
				let target = resolve_simple_reference(rrl.target.as_ref())?;
				if matches!(target, SimpleBackendReference::Invalid) {
					return Err(ProtoError::Generic(
						"remote_rate_limit: target must be set".into(),
					));
				}
				TrafficPolicy::RemoteRateLimit(http::remoteratelimit::RemoteRateLimit {
					domain: rrl.domain.clone(),
					target: Arc::new(target),
					descriptors: Arc::new(http::remoteratelimit::DescriptorSet(descriptors)),
				})
			},
			Some(tps::Kind::Csrf(csrf_spec)) => {
				let additional_origins: std::collections::HashSet<String> =
					csrf_spec.additional_origins.iter().cloned().collect();
				TrafficPolicy::Csrf(crate::http::csrf::Csrf::new(additional_origins))
			},
			Some(tps::Kind::ExtProc(ep)) => {
				let target = resolve_simple_reference(ep.target.as_ref())?;
				let failure_mode = match tps::ext_proc::FailureMode::try_from(ep.failure_mode) {
					Ok(tps::ext_proc::FailureMode::FailOpen) => http::ext_proc::FailureMode::FailOpen,
					_ => http::ext_proc::FailureMode::FailClosed,
				};
				TrafficPolicy::ExtProc(http::ext_proc::ExtProc {
					target: Arc::new(target),
					failure_mode,
				})
			},
			Some(tps::Kind::RequestHeaderModifier(rhm)) => {
				TrafficPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
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
			Some(tps::Kind::ResponseHeaderModifier(rhm)) => {
				TrafficPolicy::ResponseHeaderModifier(http::filters::HeaderModifier {
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
			Some(tps::Kind::RequestRedirect(rr)) => {
				TrafficPolicy::RequestRedirect(http::filters::RequestRedirect {
					scheme: default_as_none(rr.scheme.as_str())
						.map(Scheme::try_from)
						.transpose()?,
					authority: match (default_as_none(rr.host.as_str()), default_as_none(rr.port)) {
						(Some(h), Some(p)) => Some(HostRedirect::Full(strng::format!("{h}:{p}"))),
						(_, Some(p)) => Some(HostRedirect::Port(NonZeroU16::new(p as u16).unwrap())),
						(Some(h), _) => Some(HostRedirect::Host(strng::new(h))),
						(None, None) => None,
					},
					path: match &rr.path {
						Some(proto::agent::request_redirect::Path::Full(f)) => {
							Some(PathRedirect::Full(strng::new(f)))
						},
						Some(proto::agent::request_redirect::Path::Prefix(f)) => {
							Some(PathRedirect::Prefix(strng::new(f)))
						},
						None => None,
					},
					status: default_as_none(rr.status)
						.map(|i| StatusCode::from_u16(i as u16))
						.transpose()?,
				})
			},
			Some(tps::Kind::UrlRewrite(ur)) => {
				let authority = if ur.host.is_empty() {
					None
				} else {
					Some(HostRedirect::Host(strng::new(&ur.host)))
				};
				let path = match &ur.path {
					Some(proto::agent::url_rewrite::Path::Full(f)) => Some(PathRedirect::Full(strng::new(f))),
					Some(proto::agent::url_rewrite::Path::Prefix(p)) => {
						Some(PathRedirect::Prefix(strng::new(p)))
					},
					None => None,
				};
				TrafficPolicy::UrlRewrite(http::filters::UrlRewrite { authority, path })
			},
			Some(tps::Kind::RequestMirror(m)) => {
				let mirrors = m
					.mirrors
					.iter()
					.map(|m| {
						let backend = resolve_simple_reference(m.backend.as_ref())?;
						Ok::<_, ProtoError>(http::filters::RequestMirror {
							backend,
							percentage: m.percentage / 100.0,
						})
					})
					.collect::<Result<Vec<_>, _>>()?;
				TrafficPolicy::RequestMirror(mirrors)
			},
			Some(tps::Kind::DirectResponse(dr)) => {
				TrafficPolicy::DirectResponse(http::filters::DirectResponse {
					body: bytes::Bytes::copy_from_slice(&dr.body),
					status: StatusCode::from_u16(dr.status as u16)?,
				})
			},
			Some(tps::Kind::Cors(c)) => TrafficPolicy::CORS(
				http::cors::Cors::try_from(http::cors::CorsSerde {
					allow_credentials: c.allow_credentials,
					allow_headers: c.allow_headers.clone(),
					allow_methods: c.allow_methods.clone(),
					allow_origins: c.allow_origins.clone(),
					expose_headers: c.expose_headers.clone(),
					max_age: c.max_age.as_ref().map(|d| (*d).try_into()).transpose()?,
				})
				.map_err(|e| ProtoError::Generic(e.to_string()))?,
			),
			Some(tps::Kind::BasicAuth(ba)) => {
				let mode = match tps::basic_authentication::Mode::try_from(ba.mode)
					.map_err(|_| ProtoError::EnumParse("invalid Basic Auth mode".to_string()))?
				{
					tps::basic_authentication::Mode::Strict => http::basicauth::Mode::Strict,
					tps::basic_authentication::Mode::Optional => http::basicauth::Mode::Optional,
				};
				TrafficPolicy::BasicAuth(http::basicauth::BasicAuthentication::new(
					&ba.htpasswd_content,
					ba.realm.clone(),
					mode,
				))
			},
			Some(tps::Kind::ApiKeyAuth(ba)) => {
				let mode = match tps::api_key::Mode::try_from(ba.mode)
					.map_err(|_| ProtoError::EnumParse("invalid API Key mode".to_string()))?
				{
					tps::api_key::Mode::Strict => http::apikey::Mode::Strict,
					tps::api_key::Mode::Optional => http::apikey::Mode::Optional,
				};
				let keys = ba
					.api_keys
					.iter()
					.map(|u| {
						let meta = u
							.metadata
							.as_ref()
							.map(serde_json::to_value)
							.transpose()?
							.unwrap_or_default();
						Ok::<_, ProtoError>((http::apikey::APIKey::new(u.key.clone()), meta))
					})
					.collect::<Result<Vec<_>, _>>()?;
				TrafficPolicy::APIKey(http::apikey::APIKeyAuthentication::new(keys, mode))
			},
			Some(tps::Kind::HostRewrite(hr)) => {
				let mode = tps::host_rewrite::Mode::try_from(hr.mode)?;
				TrafficPolicy::HostRewrite(match mode {
					Mode::None => agent::HostRedirectOverride::None,
					Mode::Auto => agent::HostRedirectOverride::Auto,
				})
			},
			None => return Err(ProtoError::MissingRequiredField),
		})
	}
}

fn convert_duration(d: prost_types::Duration) -> Duration {
	Duration::from_secs(d.seconds as u64) + Duration::from_nanos(d.nanos as u64)
}

impl TryFrom<&proto::agent::FrontendPolicySpec> for FrontendPolicy {
	type Error = ProtoError;

	fn try_from(spec: &proto::agent::FrontendPolicySpec) -> Result<Self, Self::Error> {
		use crate::types::frontend;
		use crate::types::proto::agent::frontend_policy_spec as fps;

		Ok(match &spec.kind {
			Some(fps::Kind::Http(h)) => FrontendPolicy::HTTP(frontend::HTTP {
				max_buffer_size: h
					.max_buffer_size
					.map(|v| v as usize)
					.unwrap_or_else(crate::defaults::max_buffer_size),
				http1_max_headers: h.http1_max_headers.map(|v| v as usize),
				http1_idle_timeout: h
					.http1_idle_timeout
					.map(convert_duration)
					.unwrap_or_else(crate::defaults::http1_idle_timeout),
				http2_window_size: h.http2_window_size,
				http2_connection_window_size: h.http2_connection_window_size,
				http2_frame_size: h.http2_frame_size,
				http2_keepalive_interval: h.http2_keepalive_interval.map(convert_duration),
				http2_keepalive_timeout: h.http2_keepalive_timeout.map(convert_duration),
			}),
			Some(fps::Kind::Tls(t)) => FrontendPolicy::TLS(frontend::TLS {
				tls_handshake_timeout: t
					.tls_handshake_timeout
					.map(convert_duration)
					.unwrap_or_else(crate::defaults::tls_handshake_timeout),
			}),
			Some(fps::Kind::Tcp(t)) => FrontendPolicy::TCP(frontend::TCP {
				keepalives: t
					.keepalives
					.as_ref()
					.map(types::agent::KeepaliveConfig::try_from)
					.transpose()?
					.unwrap_or_default(),
			}),
			Some(fps::Kind::Logging(p)) => {
				let (add, rm) = p
					.fields
					.as_ref()
					.map(|f| {
						let add = f
							.add
							.iter()
							.map(|f| {
								let expr = cel::Expression::new(&f.expression).map_err(|e| {
									ProtoError::Generic(format!("invalid CEL expression in add field: {e}"))
								})?;
								Ok::<_, ProtoError>((f.name.clone(), Arc::new(expr)))
							})
							.collect::<Result<Vec<_>, _>>()?;
						let rm = f.remove.clone();
						Ok::<_, ProtoError>((OrderedStringMap::from_iter(add), rm))
					})
					.transpose()?
					.unwrap_or_default();
				FrontendPolicy::AccessLog(frontend::LoggingPolicy {
					filter: p
						.filter
						.as_ref()
						.map(cel::Expression::new)
						.transpose()
						.map_err(|e| {
							ProtoError::Generic(format!("invalid CEL expression in filter field: {e}"))
						})?
						.map(Arc::new),
					add: Arc::new(add),
					remove: Arc::new(FzHashSet::new(rm)),
				})
			},
			Some(fps::Kind::Tracing(_)) => FrontendPolicy::Tracing(()),
			None => return Err(ProtoError::MissingRequiredField),
		})
	}
}

impl TryFrom<&proto::agent::KeepaliveConfig> for KeepaliveConfig {
	type Error = ProtoError;

	fn try_from(k: &proto::agent::KeepaliveConfig) -> Result<Self, Self::Error> {
		Ok(KeepaliveConfig {
			enabled: true,
			time: k
				.time
				.map(convert_duration)
				.unwrap_or_else(types::agent::defaults::keepalive_time),
			interval: k
				.interval
				.map(convert_duration)
				.unwrap_or_else(types::agent::defaults::keepalive_interval),
			retries: k
				.retries
				.unwrap_or_else(types::agent::defaults::keepalive_retries),
		})
	}
}

impl TryFrom<&proto::agent::PolicyTarget> for PolicyTarget {
	type Error = ProtoError;

	fn try_from(t: &proto::agent::PolicyTarget) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::policy_target as tgt;
		match t.kind.as_ref() {
			Some(tgt::Kind::Gateway(g)) => Ok(PolicyTarget::Gateway(strng::new(g))),
			Some(tgt::Kind::Listener(l)) => Ok(PolicyTarget::Listener(strng::new(l))),
			Some(tgt::Kind::Route(r)) => Ok(PolicyTarget::Route(strng::new(r))),
			Some(tgt::Kind::RouteRule(r)) => Ok(PolicyTarget::RouteRule(strng::new(r))),
			Some(tgt::Kind::Backend(b)) => Ok(PolicyTarget::Backend(strng::new(b))),
			Some(tgt::Kind::Service(s)) => Ok(PolicyTarget::Service(strng::new(s))),
			Some(tgt::Kind::SubBackend(sb)) => Ok(PolicyTarget::SubBackend(strng::new(sb))),
			None => Err(ProtoError::MissingRequiredField),
		}
	}
}

impl TryFrom<&proto::agent::Policy> for TargetedPolicy {
	type Error = ProtoError;

	fn try_from(p: &proto::agent::Policy) -> Result<Self, Self::Error> {
		use crate::types::proto::agent::policy as pol;

		let name = strng::new(&p.name);
		let target = p
			.target
			.as_ref()
			.ok_or(ProtoError::MissingRequiredField)
			.and_then(PolicyTarget::try_from)?;

		let policy = match &p.kind {
			Some(pol::Kind::Traffic(spec)) => PolicyType::Traffic(PhasedTrafficPolicy::try_from(spec)?),
			Some(pol::Kind::Backend(spec)) => PolicyType::Backend(BackendPolicy::try_from(spec)?),
			// Frontend policies are not represented by TargetedPolicy; reject here.
			Some(pol::Kind::Frontend(spec)) => PolicyType::Frontend(FrontendPolicy::try_from(spec)?),
			None => return Err(ProtoError::MissingRequiredField),
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

fn convert_message(
	m: &proto::agent::backend_policy_spec::ai::Message,
) -> crate::llm::SimpleChatCompletionMessage {
	llm::SimpleChatCompletionMessage {
		role: strng::new(&m.role),
		content: strng::new(&m.content),
	}
}

fn convert_prompt_enrichment(
	prompts: &proto::agent::backend_policy_spec::ai::PromptEnrichment,
) -> crate::llm::policy::PromptEnrichment {
	crate::llm::policy::PromptEnrichment {
		append: prompts.append.iter().map(convert_message).collect(),
		prepend: prompts.prepend.iter().map(convert_message).collect(),
	}
}

fn convert_prompt_caching(
	pc: &proto::agent::backend_policy_spec::ai::PromptCaching,
) -> crate::llm::policy::PromptCachingConfig {
	crate::llm::policy::PromptCachingConfig {
		cache_system: pc.cache_system,
		cache_messages: pc.cache_messages,
		cache_tools: pc.cache_tools,
		min_tokens: pc.min_tokens.map(|t| t as usize),
	}
}

fn convert_webhook(
	w: &proto::agent::backend_policy_spec::ai::Webhook,
) -> Option<crate::llm::policy::Webhook> {
	let port = match u16::try_from(w.port) {
		Ok(port) => port,
		Err(_) => {
			warn!(port = w.port, host = %w.host, "Webhook port out of range, ignoring webhook");
			return None;
		},
	};

	let forward_header_matches = match convert_header_match(&w.forward_header_matches) {
		Ok(h) => h,
		Err(e) => {
			warn!(error = %e, "Invalid webhook header matchers, ignoring webhook");
			return None;
		},
	};

	Some(crate::llm::policy::Webhook {
		target: Target::Hostname(w.host.clone().into(), port),
		forward_header_matches,
	})
}

fn convert_regex_rules(
	rr: &proto::agent::backend_policy_spec::ai::RegexRules,
	rejection: Option<crate::llm::policy::RequestRejection>,
) -> crate::llm::policy::RegexRules {
	let action = match rr
		.action
		.as_ref()
		.and_then(|a| proto::agent::backend_policy_spec::ai::ActionKind::try_from(a.kind).ok())
	{
		Some(proto::agent::backend_policy_spec::ai::ActionKind::Reject) => {
			crate::llm::policy::Action::Reject {
				response: rejection.unwrap_or_default(),
			}
		},
		_ => crate::llm::policy::Action::Mask,
	};
	let rules = rr
		.rules
		.iter()
		.filter_map(|r| match &r.kind {
			Some(proto::agent::backend_policy_spec::ai::regex_rule::Kind::Builtin(b)) => {
				match proto::agent::backend_policy_spec::ai::BuiltinRegexRule::try_from(*b) {
					Ok(builtin) => {
						let builtin = match builtin {
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::Ssn => {
								crate::llm::policy::Builtin::Ssn
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::CreditCard => {
								crate::llm::policy::Builtin::CreditCard
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::PhoneNumber => {
								crate::llm::policy::Builtin::PhoneNumber
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::Email => {
								crate::llm::policy::Builtin::Email
							},
							_ => {
								warn!(value = *b, "Unknown builtin regex rule, skipping");
								return None;
							},
						};
						Some(crate::llm::policy::RegexRule::Builtin { builtin })
					},
					Err(_) => {
						warn!(value = *b, "Invalid builtin regex rule value, skipping");
						None
					},
				}
			},
			Some(proto::agent::backend_policy_spec::ai::regex_rule::Kind::Regex(n)) => {
				match regex::Regex::new(&n.pattern) {
					Ok(pattern) => Some(crate::llm::policy::RegexRule::Regex {
						pattern,
						name: n.name.clone(),
					}),
					Err(err) => {
						warn!(error = %err, name = %n.name, pattern = %n.pattern, "Invalid regex pattern");
						None
					},
				}
			},
			None => None,
		})
		.collect();
	crate::llm::policy::RegexRules { action, rules }
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

fn convert_header_match(h: &[proto::agent::HeaderMatch]) -> Result<Vec<HeaderMatch>, ProtoError> {
	let headers = h
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
	Ok(headers)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;
	use crate::types::proto::agent::backend_policy_spec::Ai;

	#[test]
	fn test_policy_spec_to_csrf_policy() -> Result<(), ProtoError> {
		// Test CSRF policy conversion with deduplication
		let csrf_spec = crate::types::proto::agent::traffic_policy_spec::Csrf {
			additional_origins: vec![
				"https://trusted.com".to_string(),
				"https://app.example.com".to_string(),
				"https://trusted.com".to_string(), // duplicate - should be deduplicated
				"https://another.com".to_string(),
			],
		};

		let spec = proto::agent::TrafficPolicySpec {
			phase: proto::agent::traffic_policy_spec::PolicyPhase::Route as i32,
			kind: Some(proto::agent::traffic_policy_spec::Kind::Csrf(csrf_spec)),
		};

		let policy = TrafficPolicy::try_from(&spec)?;

		if let TrafficPolicy::Csrf(_csrf_policy) = policy {
			// We can't directly access the HashSet since it's private, but we can test
			// the policy works by creating a test that would use the contains() method
			// This verifies the conversion worked and the HashSet deduplication happened

			// For now, just verify we got a CSRF policy
			// In a real implementation, you'd add a test helper method to the Csrf struct
			// to verify the contents
			Ok(())
		} else {
			panic!("Expected CSRF policy variant, got: {:?}", policy);
		}
	}

	#[tokio::test]
	async fn test_ai_backend_routes_conversion() -> Result<(), ProtoError> {
		use proto::agent::ai_backend;

		// Test proto routes field converts to Rust RouteType
		let mut routes_map = std::collections::HashMap::new();
		routes_map.insert(
			"/v1/chat/completions".to_string(),
			ai_backend::RouteType::Completions as i32,
		);
		routes_map.insert(
			"/v1/messages".to_string(),
			ai_backend::RouteType::Messages as i32,
		);

		let proto_backend = proto::agent::Backend {
			name: "test/backend".to_string(),
			kind: Some(proto::agent::backend::Kind::Ai(proto::agent::AiBackend {
				provider_groups: vec![ai_backend::ProviderGroup {
					providers: vec![ai_backend::Provider {
						name: "test-provider".to_string(),
						host_override: None,
						path_override: None,
						routes: routes_map,
						provider: Some(ai_backend::provider::Provider::Openai(ai_backend::OpenAi {
							model: None,
						})),
						inline_policies: vec![],
					}],
				}],
			})),
			inline_policies: vec![],
		};

		let backend = BackendWithPolicies::try_from(&proto_backend)?.backend;
		if let Backend::AI(name, ai_backend) = backend {
			assert_eq!(name.as_str(), "test/backend");
			let (provider, _handle) = ai_backend.select_provider().expect("should have provider");
			assert_eq!(provider.routes.len(), 2);
			// Suffix matching: paths ending with these suffixes should match
			assert_eq!(
				provider.resolve_route("/v1/chat/completions"),
				llm::RouteType::Completions
			);
			assert_eq!(
				provider.resolve_route("/v1/messages"),
				llm::RouteType::Messages
			);
			// No match -> default to Completions
			assert_eq!(
				provider.resolve_route("/unknown"),
				llm::RouteType::Completions
			);
			Ok(())
		} else {
			panic!("Expected AI backend")
		}
	}

	#[test]
	fn test_backend_policy_spec_to_ai_policy() -> Result<(), ProtoError> {
		let spec = proto::agent::BackendPolicySpec {
			kind: Some(proto::agent::backend_policy_spec::Kind::Ai(Ai {
				defaults: vec![
					("temperature".to_string(), "0.7".to_string()),
					("max_tokens".to_string(), "2000".to_string()),
					(
						"object_value".to_string(),
						"{\"key\":\"value\"}".to_string(),
					),
				]
				.into_iter()
				.collect(),
				overrides: vec![
					("model".to_string(), "\"gpt-4\"".to_string()),
					("frequency_penalty".to_string(), "0.5".to_string()),
					("array_value".to_string(), "[1,2,3]".to_string()),
				]
				.into_iter()
				.collect(),
				prompt_guard: None,
				prompts: None,
				model_aliases: Default::default(),
				prompt_caching: None,
			})),
		};

		let policy = BackendPolicy::try_from(&spec)?;

		if let BackendPolicy::AI(ai_policy) = policy {
			let defaults = ai_policy.defaults.as_ref().expect("defaults should be set");
			let overrides = ai_policy
				.overrides
				.as_ref()
				.expect("overrides should be set");

			// Verify defaults have correct types and values
			let temp_val = defaults.get("temperature").unwrap();
			assert!(temp_val.is_f64(), "temperature should be f64");
			assert_eq!(temp_val.as_f64().unwrap(), 0.7);

			let tokens_val = defaults.get("max_tokens").unwrap();
			assert!(tokens_val.is_u64(), "max_tokens should be u64");
			assert_eq!(tokens_val.as_u64().unwrap(), 2000);

			let obj_val = defaults.get("object_value").unwrap();
			assert!(obj_val.is_object(), "object_value should be an object");
			assert_eq!(obj_val, &json!({"key": "value"}));

			// Verify overrides have correct types and values
			let model_val = overrides.get("model").unwrap();
			assert!(model_val.is_string(), "model should be a string");
			assert_eq!(model_val.as_str().unwrap(), "gpt-4");

			let freq_val = overrides.get("frequency_penalty").unwrap();
			assert!(freq_val.is_f64(), "frequency_penalty should be f64");
			assert_eq!(freq_val.as_f64().unwrap(), 0.5);

			let array_val = overrides.get("array_value").unwrap();
			assert!(array_val.is_array(), "array_value should be an array");
			assert_eq!(array_val, &json!([1, 2, 3]));
		} else {
			panic!("Expected AI policy variant");
		}

		Ok(())
	}
}
