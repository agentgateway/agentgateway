//! agent_xds defines the translation from XDS protobufs into our internal representation.
//! Error handling in translation is important. Rejecting a configuration can have catastrophic impacts
//! (a port 443 bind rejection could take down an entire site).
//! We take the following approach:
//! * We distinguish between Errors and Warnings. Since XDS doesn't have this concept, we return both of
//!   these as errors in the XDS layer, but a warning applies the resource *and then* errors.
//! * If something is invalid as result of an invalid user configuration (such as a bad CEL expression, bad Regex, etc)
//!   we ought to treat it as a warning.
//!   * When we warn, we should fail in context-specific ways. For example, a Regex route matcher may never match.
//!     An invalid backend may always return a 5xx error. A bad ext_authz config may allow or deny all requests, following
//!     configured `failureMode` semantics.
//! * If something is entirely invalid, such as sending an unknown enum, etc, we currently treat these as errors.
//! * We aim to generally specialize for the native Go agentgateway control plane. What may be impossible
//!   to happen in Agentgateway's controller may be possible with a third-party controller which may
//!   make the error/warning distinction not fully respected.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;

use ::http::{HeaderName, StatusCode};
use frozen_collections::FzHashSet;
use itertools::Itertools;
use llm::{AIBackend, AIProvider, NamedAIProvider};

use super::agent::*;
use crate::http::auth::{AwsAuth, BackendAuth, GcpAuth};
use crate::http::oidc::OidcCookieEncoder;
use crate::http::transformation_cel::{LocalTransform, LocalTransformationConfig, Transformation};
use crate::http::{HeaderOrPseudo, Scheme, auth, authorization, health};
use crate::mcp::{FailureMode, McpAuthorization};
use crate::store::RequestPolicy;
use crate::telemetry::log::OrderedStringMap;
use crate::types::discovery::NamespacedHostname;
use crate::types::proto::ProtoError;
use crate::types::proto::agent::backend_policy_spec::ai::request_guard::Kind;
use crate::types::proto::agent::backend_policy_spec::ai::{ActionKind, response_guard};
use crate::types::proto::agent::backend_policy_spec::backend_http::HttpVersion;
use crate::types::proto::agent::mcp_target::Protocol;
use crate::types::proto::agent::traffic_policy_spec::host_rewrite::Mode;
use crate::types::{agent, backend, proto};
use crate::*;

#[derive(Debug, Default)]
pub struct Diagnostics {
	warnings: Vec<String>,
}

impl Diagnostics {
	pub fn add_warning(&mut self, warning: impl Into<String>) {
		self.warnings.push(warning.into());
	}

	pub fn is_empty(&self) -> bool {
		self.warnings.is_empty()
	}

	pub fn into_warnings(self) -> Vec<String> {
		self.warnings
	}
}

fn permissive_cel_expression(
	diagnostics: &mut Diagnostics,
	context: impl AsRef<str>,
	original_expression: impl Into<String>,
) -> cel::Expression {
	let original_expression = original_expression.into();
	let (expression, err) = cel::Expression::new_permissive(original_expression.clone());
	if let Some(err) = err {
		diagnostics.add_warning(format!(
			"invalid CEL expression for {}: {err}; replacing {original_expression:?} with an expression that always fails",
			context.as_ref(),
		));
	}
	expression
}

fn permissive_cel_expression_arc(
	diagnostics: &mut Diagnostics,
	context: impl AsRef<str>,
	original_expression: impl Into<String>,
) -> Arc<cel::Expression> {
	Arc::new(permissive_cel_expression(
		diagnostics,
		context,
		original_expression,
	))
}

fn regex_or_warn_invalid(
	diagnostics: &mut Diagnostics,
	context: impl AsRef<str>,
	pattern: &str,
) -> Result<regex::Regex, regex::Error> {
	regex::Regex::new(pattern).inspect_err(|err| {
		diagnostics.add_warning(format!(
			"invalid regex for {}: {err}; replacing {pattern:?} with a matcher that never matches",
			context.as_ref(),
		));
	})
}

fn convert_tls_cipher_suites(
	raw_suites: &[i32],
	diagnostics: &mut Diagnostics,
) -> Option<Vec<crate::transport::tls::CipherSuite>> {
	if raw_suites.is_empty() {
		return None;
	}

	let mut out = Vec::with_capacity(raw_suites.len());
	for &raw in raw_suites {
		if raw == 0 {
			// CIPHER_SUITE_UNSPECIFIED
			continue;
		}
		match proto::agent::tls_config::CipherSuite::try_from(raw) {
			Ok(suite) => match crate::transport::tls::CipherSuite::try_from(suite) {
				Ok(suite) => out.push(suite),
				Err(e) => {
					diagnostics.add_warning(format!("unknown/unsupported TLS cipher suite {raw}: {e}"));
				},
			},
			Err(e) => {
				diagnostics.add_warning(format!("unknown TLS cipher suite enum value {raw}: {e}"));
			},
		}
	}
	if out.is_empty() { None } else { Some(out) }
}

fn convert_tls_key_exchange_groups(
	raw_groups: &[i32],
	diagnostics: &mut Diagnostics,
) -> Option<Vec<crate::transport::tls::KeyExchangeGroup>> {
	if raw_groups.is_empty() {
		return None;
	}

	let mut out = Vec::with_capacity(raw_groups.len());
	for &raw in raw_groups {
		if raw == 0 {
			// KEY_EXCHANGE_GROUP_UNSPECIFIED
			continue;
		}
		match proto::agent::tls_config::KeyExchangeGroup::try_from(raw) {
			Ok(group) => match crate::transport::tls::KeyExchangeGroup::try_from(group) {
				Ok(group) => out.push(group),
				Err(e) => {
					diagnostics.add_warning(format!(
						"unknown/unsupported TLS key exchange group {raw}: {e}"
					));
				},
			},
			Err(e) => {
				diagnostics.add_warning(format!(
					"unknown TLS key exchange group enum value {raw}: {e}"
				));
			},
		}
	}
	if out.is_empty() { None } else { Some(out) }
}

impl TryFrom<proto::agent::tls_config::CipherSuite> for crate::transport::tls::CipherSuite {
	type Error = anyhow::Error;

	fn try_from(value: proto::agent::tls_config::CipherSuite) -> Result<Self, Self::Error> {
		use crate::transport::tls::CipherSuite as Cs;
		match value {
			proto::agent::tls_config::CipherSuite::Unspecified => Err(anyhow::anyhow!(
				"unsupported cipher suite: CIPHER_SUITE_UNSPECIFIED"
			)),
			proto::agent::tls_config::CipherSuite::TlsAes256GcmSha384 => Ok(Cs::TLS_AES_256_GCM_SHA384),
			proto::agent::tls_config::CipherSuite::TlsAes128GcmSha256 => Ok(Cs::TLS_AES_128_GCM_SHA256),
			proto::agent::tls_config::CipherSuite::TlsChacha20Poly1305Sha256 => {
				Ok(Cs::TLS_CHACHA20_POLY1305_SHA256)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheEcdsaWithAes256GcmSha384 => {
				Ok(Cs::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheEcdsaWithAes128GcmSha256 => {
				Ok(Cs::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheEcdsaWithChacha20Poly1305Sha256 => {
				Ok(Cs::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheRsaWithAes256GcmSha384 => {
				Ok(Cs::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheRsaWithAes128GcmSha256 => {
				Ok(Cs::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256)
			},
			proto::agent::tls_config::CipherSuite::TlsEcdheRsaWithChacha20Poly1305Sha256 => {
				Ok(Cs::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256)
			},
		}
	}
}

impl TryFrom<proto::agent::tls_config::KeyExchangeGroup>
	for crate::transport::tls::KeyExchangeGroup
{
	type Error = anyhow::Error;

	fn try_from(value: proto::agent::tls_config::KeyExchangeGroup) -> Result<Self, Self::Error> {
		use crate::transport::tls::KeyExchangeGroup as Kx;
		match value {
			proto::agent::tls_config::KeyExchangeGroup::Unspecified => Err(anyhow::anyhow!(
				"unsupported key exchange group: KEY_EXCHANGE_GROUP_UNSPECIFIED"
			)),
			proto::agent::tls_config::KeyExchangeGroup::X25519 => Ok(Kx::X25519),
			proto::agent::tls_config::KeyExchangeGroup::P256 => Ok(Kx::P256),
			proto::agent::tls_config::KeyExchangeGroup::P384 => Ok(Kx::P384),
			proto::agent::tls_config::KeyExchangeGroup::X25519Mlkem768 => Ok(Kx::X25519_MLKEM768),
		}
	}
}

fn server_tls_config_from_proto(
	value: &proto::agent::TlsConfig,
	diagnostics: &mut Diagnostics,
) -> ServerTLSConfig {
	// Defaults set here. These can be overridden by Frontend policy
	// TODO: this default only makes sense for HTTPS, distinguish from TLS
	let default_alpns = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

	// These are optional, so treat unknown/unsupported values as "unset".
	// rustls in this repo only supports TLS1.2/1.3.
	let map_tls_version = |raw: Option<i32>| {
		raw.and_then(
			|raw| match proto::agent::tls_config::TlsVersion::try_from(raw).ok() {
				Some(proto::agent::tls_config::TlsVersion::TlsV12) => Some(TLSVersion::TLS_V1_2),
				Some(proto::agent::tls_config::TlsVersion::TlsV13) => Some(TLSVersion::TLS_V1_3),
				_ => None,
			},
		)
	};
	let min_version = map_tls_version(value.min_version);
	let max_version = map_tls_version(value.max_version);
	let cipher_suites = convert_tls_cipher_suites(&value.cipher_suites, diagnostics);
	let key_exchange_groups =
		convert_tls_key_exchange_groups(&value.key_exchange_groups, diagnostics);

	let mtls_mode = proto::agent::tls_config::MtlsMode::try_from(value.mtls_mode).unwrap_or_default();
	let certificate_source =
		proto::agent::tls_config::CertificateSource::try_from(value.certificate_source)
			.unwrap_or_default();

	if certificate_source == proto::agent::tls_config::CertificateSource::IstioWorkload {
		let require_client_cert = match mtls_mode {
			proto::agent::tls_config::MtlsMode::Strict => true,
			proto::agent::tls_config::MtlsMode::Disable => false,
			proto::agent::tls_config::MtlsMode::AllowInsecureFallback => {
				diagnostics.add_warning(
					"ALLOW_INSECURE_FALLBACK is not supported with ISTIO_WORKLOAD certificates; disabling mTLS",
				);
				false
			},
		};
		return ServerTLSConfig::istio_workload(require_client_cert, default_alpns);
	}

	match ServerTLSConfig::from_pem_with_profile(
		value.cert.clone(),
		value.private_key.clone(),
		value.root.clone(),
		default_alpns,
		min_version,
		max_version,
		cipher_suites,
		key_exchange_groups,
		mtls_mode == proto::agent::tls_config::MtlsMode::AllowInsecureFallback,
	) {
		Ok(sc) => sc,
		Err(e) => {
			diagnostics.add_warning(format!("TLS certificate is invalid: {e}"));
			ServerTLSConfig::new_invalid()
		},
	}
}

fn route_backend_reference_from_proto(
	s: &proto::agent::RouteBackend,
	diagnostics: &mut Diagnostics,
) -> Result<RouteBackendReference, ProtoError> {
	let inline_policies = s
		.backend_policies
		.iter()
		.map(|spec| backend_policy_from_proto(spec, diagnostics))
		.collect::<Result<Vec<_>, _>>()?;
	let target = if let Some(rgk) = s.route_group_key.as_ref() {
		RouteBackendTarget::RouteGroup(strng::new(rgk))
	} else {
		let backend = resolve_reference(s.backend.as_ref());
		backend.into()
	};
	Ok(RouteBackendReference {
		weight: s.weight as usize,
		target,
		inline_policies,
	})
}

fn mcp_authorization_from_proto(
	rbac: &proto::agent::backend_policy_spec::McpAuthorization,
	diagnostics: &mut Diagnostics,
) -> McpAuthorization {
	let mut allow_exprs = Vec::new();
	// We do NOT want to NACK invalid CEL expressions. Instead, we ensure they always evaluate to errors.
	for allow_rule in &rbac.allow {
		allow_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"backend.mcpAuthorization.allow",
			allow_rule,
		));
	}

	let mut deny_exprs = Vec::new();
	for deny_rule in &rbac.deny {
		deny_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"backend.mcpAuthorization.deny",
			deny_rule,
		));
	}

	let mut require_exprs = Vec::new();
	for require_rule in &rbac.require {
		require_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"backend.mcpAuthorization.require",
			require_rule,
		));
	}

	let policy_set = authorization::PolicySet::new(allow_exprs, deny_exprs, require_exprs);
	McpAuthorization::new(authorization::RuleSet::new(policy_set))
}

fn mcp_authentication_from_proto(
	m: &proto::agent::backend_policy_spec::McpAuthentication,
	_diagnostics: &mut Diagnostics,
) -> Result<McpAuthentication, ProtoError> {
	if m.jwks_inline.is_empty() {
		return Err(ProtoError::Generic(
			"MCP Authentication requires jwks_inline to be set.".to_string(),
		));
	}
	let jwks_json = m.jwks_inline.clone();

	let jwk_set: jsonwebtoken::jwk::JwkSet = serde_json::from_str(&jwks_json).map_err(|e| {
		ProtoError::Generic(format!("failed to parse JWKS for MCP Authentication: {e}"))
	})?;

	let audiences = (!m.audiences.is_empty()).then(|| m.audiences.clone());
	let jwt_validation_options = m
		.jwt_validation_options
		.as_ref()
		.map(|vo| http::jwt::JWTValidationOptions {
			required_claims: vo.required_claims.iter().cloned().collect(),
		})
		.unwrap_or_default();
	let jwt_provider =
		http::jwt::Provider::from_jwks(jwk_set, m.issuer.clone(), audiences, jwt_validation_options)
			.map_err(|e| {
				ProtoError::Generic(format!(
					"failed to create JWT provider for MCP Authentication: {e}"
				))
			})?;

	let mode = match proto::agent::backend_policy_spec::mcp_authentication::Mode::try_from(m.mode)
		.map_err(|_| ProtoError::EnumParse("invalid JWT mode".to_string()))?
	{
		proto::agent::backend_policy_spec::mcp_authentication::Mode::Optional => {
			McpAuthenticationMode::Optional
		},
		proto::agent::backend_policy_spec::mcp_authentication::Mode::Strict => {
			McpAuthenticationMode::Strict
		},
		proto::agent::backend_policy_spec::mcp_authentication::Mode::Permissive => {
			McpAuthenticationMode::Permissive
		},
	};

	let jwt_validator = http::jwt::Jwt::from_providers(
		vec![jwt_provider],
		mode.into(),
		http::auth::AuthorizationLocation::bearer_header(),
	);
	Ok(build_mcp_authentication(
		m.issuer.clone(),
		m.audiences.clone(),
		m.provider,
		convert_mcp_resource_metadata(m.resource_metadata.as_ref().map(|rm| rm.extra.iter())),
		std::sync::Arc::new(jwt_validator),
		mode,
	))
}

fn convert_mcp_provider(provider: i32) -> Option<McpIDP> {
	use proto::agent::backend_policy_spec::mcp_authentication::McpIdp;
	match provider {
		x if x == McpIdp::Unspecified as i32 => None,
		x if x == McpIdp::Auth0 as i32 => Some(McpIDP::Auth0 {}),
		x if x == McpIdp::Keycloak as i32 => Some(McpIDP::Keycloak {}),
		_ => None,
	}
}

fn convert_mcp_resource_metadata<'a, I, V>(entries: Option<I>) -> ResourceMetadata
where
	I: IntoIterator<Item = (&'a String, &'a V)>,
	V: serde::Serialize + 'a,
{
	let extra = entries
		.map(|entries| {
			entries
				.into_iter()
				.map(|(key, value)| {
					let value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
					(key.clone(), value)
				})
				.collect()
		})
		.unwrap_or_default();
	ResourceMetadata { extra }
}

fn build_mcp_authentication(
	issuer: String,
	audiences: Vec<String>,
	provider: i32,
	resource_metadata: ResourceMetadata,
	jwt_validator: Arc<http::jwt::Jwt>,
	mode: McpAuthenticationMode,
) -> McpAuthentication {
	McpAuthentication {
		issuer,
		audiences,
		provider: convert_mcp_provider(provider),
		resource_metadata,
		jwt_validator,
		mode,
	}
}

fn convert_route_type(proto_rt: i32, diagnostics: &mut Diagnostics) -> llm::RouteType {
	use proto::agent::backend_policy_spec::ai::RouteType as ProtoRT;

	match ProtoRT::try_from(proto_rt) {
		Ok(ProtoRT::Completions) | Ok(ProtoRT::Unspecified) => llm::RouteType::Completions,
		Ok(ProtoRT::Messages) => llm::RouteType::Messages,
		Ok(ProtoRT::Models) => llm::RouteType::Models,
		Ok(ProtoRT::Passthrough) => llm::RouteType::Passthrough,
		Ok(ProtoRT::Detect) => llm::RouteType::Detect,
		Ok(ProtoRT::Responses) => llm::RouteType::Responses,
		Ok(ProtoRT::AnthropicTokenCount) => llm::RouteType::AnthropicTokenCount,
		Ok(ProtoRT::Embeddings) => llm::RouteType::Embeddings,
		Ok(ProtoRT::Realtime) => llm::RouteType::Realtime,
		Err(_) => {
			diagnostics.add_warning(format!(
				"unknown proto RouteType value {}, defaulting to Completions",
				proto_rt
			));
			llm::RouteType::Completions
		},
	}
}

fn convert_backend_ai_policy(
	ai: &proto::agent::backend_policy_spec::Ai,
	diagnostics: &mut Diagnostics,
) -> Result<llm::Policy, ProtoError> {
	let prompt_guard: Option<Result<_, ProtoError>> = ai.prompt_guard.as_ref().map(|pg| {
		let request = pg
			.request
			.iter()
			.map(|reqp| {
				let rejection = if let Some(resp) = &reqp.rejection {
					let status = u16::try_from(resp.status)
						.ok()
						.and_then(|c| StatusCode::from_u16(c).ok())
						.unwrap_or(StatusCode::FORBIDDEN);
					llm::policy::RequestRejection {
						body: Bytes::from(resp.body.clone()),
						status,
						headers: None, // TODO: map from proto if headers are added there
					}
				} else {
					//  use default response, since the response field is not optional on RequestGuard
					llm::policy::RequestRejection::default()
				};

				let kind = match reqp
					.kind
					.as_ref()
					.ok_or_else(|| ProtoError::EnumParse("unknown kind".to_string()))?
				{
					Kind::Regex(rr) => {
						llm::policy::RequestGuardKind::Regex(convert_regex_rules(rr, diagnostics))
					},
					Kind::Webhook(wh) => {
						llm::policy::RequestGuardKind::Webhook(convert_webhook(wh, diagnostics)?)
					},
					Kind::OpenaiModeration(m) => {
						let pols = m
							.inline_policies
							.iter()
							.map(|policy| backend_policy_from_proto(policy, diagnostics))
							.collect::<Result<Vec<_>, _>>()?;
						let md = llm::policy::Moderation {
							model: m.model.as_deref().map(strng::new),
							policies: pols,
						};
						llm::policy::RequestGuardKind::OpenAIModeration(md)
					},
					Kind::GoogleModelArmor(gma) => {
						let pols = gma
							.inline_policies
							.iter()
							.map(|policy| backend_policy_from_proto(policy, diagnostics))
							.collect::<Result<Vec<_>, _>>()?;
						llm::policy::RequestGuardKind::GoogleModelArmor(llm::policy::GoogleModelArmor {
							template_id: strng::new(&gma.template_id),
							project_id: strng::new(&gma.project_id),
							location: gma.location.as_ref().map(strng::new),
							policies: pols,
						})
					},
					Kind::BedrockGuardrails(bg) => {
						let pols = bg
							.inline_policies
							.iter()
							.map(|policy| backend_policy_from_proto(policy, diagnostics))
							.collect::<Result<Vec<_>, _>>()?;
						llm::policy::RequestGuardKind::BedrockGuardrails(llm::policy::BedrockGuardrails {
							guardrail_identifier: strng::new(&bg.identifier),
							guardrail_version: strng::new(&bg.version),
							region: strng::new(&bg.region),
							policies: pols,
						})
					},
					Kind::AzureContentSafety(acs) => {
						let pols = acs
							.inline_policies
							.iter()
							.map(|policy| backend_policy_from_proto(policy, diagnostics))
							.collect::<Result<Vec<_>, _>>()?;
						llm::policy::RequestGuardKind::AzureContentSafety(llm::policy::AzureContentSafety {
							endpoint: strng::new(&acs.endpoint),
							policies: pols,
							cached_azure_auth: Default::default(),
							analyze_text: Some(llm::policy::AnalyzeTextConfig {
								severity_threshold: acs.severity_threshold,
								api_version: acs.api_version.as_ref().map(strng::new),
								blocklist_names: if acs.blocklist_names.is_empty() {
									None
								} else {
									Some(acs.blocklist_names.clone())
								},
								halt_on_blocklist_hit: acs.halt_on_blocklist_hit,
							}),
							detect_jailbreak: None,
						})
					},
				};
				Ok(llm::policy::RequestGuard { rejection, kind })
			})
			.collect::<Result<Vec<_>, ProtoError>>()?;

		let response = pg.response.iter().flat_map(|reqp| {
			let rejection = if let Some(resp) = &reqp.rejection {
				let status = u16::try_from(resp.status)
					.ok()
					.and_then(|c| StatusCode::from_u16(c).ok())
					.unwrap_or(StatusCode::FORBIDDEN);
				llm::policy::RequestRejection {
					body: Bytes::from(resp.body.clone()),
					status,
					headers: None, // TODO: map from proto if headers are added there
				}
			} else {
				//  use default response, since the response field is not optional on RequestGuard
				llm::policy::RequestRejection::default()
			};

			let kind = match reqp.kind.as_ref()? {
				response_guard::Kind::Regex(rr) => {
					llm::policy::ResponseGuardKind::Regex(convert_regex_rules(rr, diagnostics))
				},
				response_guard::Kind::Webhook(wh) => {
					llm::policy::ResponseGuardKind::Webhook(convert_webhook(wh, diagnostics).ok()?)
				},
				response_guard::Kind::GoogleModelArmor(gma) => {
					let pols = gma
						.inline_policies
						.iter()
						.filter_map(|p| backend_policy_from_proto(p, diagnostics).ok())
						.collect::<Vec<_>>();
					llm::policy::ResponseGuardKind::GoogleModelArmor(llm::policy::GoogleModelArmor {
						template_id: strng::new(&gma.template_id),
						project_id: strng::new(&gma.project_id),
						location: gma.location.as_ref().map(strng::new),
						policies: pols,
					})
				},
				response_guard::Kind::BedrockGuardrails(bg) => {
					let pols = bg
						.inline_policies
						.iter()
						.filter_map(|p| backend_policy_from_proto(p, diagnostics).ok())
						.collect::<Vec<_>>();
					llm::policy::ResponseGuardKind::BedrockGuardrails(llm::policy::BedrockGuardrails {
						guardrail_identifier: strng::new(&bg.identifier),
						guardrail_version: strng::new(&bg.version),
						region: strng::new(&bg.region),
						policies: pols,
					})
				},
				response_guard::Kind::AzureContentSafety(acs) => {
					let pols = acs
						.inline_policies
						.iter()
						.filter_map(|p| backend_policy_from_proto(p, diagnostics).ok())
						.collect::<Vec<_>>();
					llm::policy::ResponseGuardKind::AzureContentSafety(llm::policy::AzureContentSafety {
						endpoint: strng::new(&acs.endpoint),
						policies: pols,
						cached_azure_auth: Default::default(),
						analyze_text: Some(llm::policy::AnalyzeTextConfig {
							severity_threshold: acs.severity_threshold,
							api_version: acs.api_version.as_ref().map(strng::new),
							blocklist_names: if acs.blocklist_names.is_empty() {
								None
							} else {
								Some(acs.blocklist_names.clone())
							},
							halt_on_blocklist_hit: acs.halt_on_blocklist_hit,
						}),
						detect_jailbreak: None,
					})
				},
			};
			Some(llm::policy::ResponseGuard { rejection, kind })
		});

		Ok(llm::policy::PromptGuard {
			request,
			response: response.collect_vec(),
		})
	});

	let mut policy = llm::Policy {
		prompt_guard: prompt_guard.transpose()?,
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
		transformations: if ai.transformations.is_empty() {
			None
		} else {
			Some(
				ai.transformations
					.iter()
					.map(|(k, v)| {
						let ve = permissive_cel_expression_arc(
							diagnostics,
							format!("backend.ai.transformations.{k}"),
							v,
						);
						Ok::<_, ProtoError>((k.to_owned(), ve))
					})
					.collect::<Result<_, _>>()?,
			)
		},
		prompts: ai.prompts.as_ref().map(convert_prompt_enrichment),
		model_aliases: ai
			.model_aliases
			.iter()
			.map(|(k, v)| (strng::new(k), strng::new(v)))
			.collect(),
		wildcard_patterns: Arc::new(Vec::new()), // Will be populated by compile_model_alias_patterns()
		prompt_caching: ai.prompt_caching.as_ref().map(convert_prompt_caching),
		routes: ai
			.routes
			.iter()
			.map(|(k, v)| (strng::new(k), convert_route_type(*v, diagnostics)))
			.collect(),
	};

	// Compile wildcard patterns from model_aliases
	policy.compile_model_alias_patterns();

	Ok(policy)
}

fn backend_auth_from_proto(
	s: proto::agent::BackendAuthPolicy,
	_diagnostics: &mut Diagnostics,
) -> Result<BackendAuth, ProtoError> {
	use proto::agent::azure_managed_identity_credential::user_assigned_identity;
	use proto::agent::{azure_explicit_config, gcp};
	Ok(match s.kind {
		Some(proto::agent::backend_auth_policy::Kind::Passthrough(p)) => BackendAuth::Passthrough {
			location: optional_authorization_location(p.authorization_location.as_ref())?,
		},
		Some(proto::agent::backend_auth_policy::Kind::Key(k)) => BackendAuth::Key {
			value: k.secret.into(),
			location: optional_authorization_location(k.authorization_location.as_ref())?,
		},
		Some(proto::agent::backend_auth_policy::Kind::Gcp(g)) => {
			let credential = g
				.credential
				.map(|credential| auth::gcp::GcpCredential::new(credential.into()))
				.transpose()
				.map_err(|e| ProtoError::Generic(e.to_string()))?;
			BackendAuth::Gcp(match g.token_type {
				None | Some(gcp::TokenType::AccessToken(gcp::AccessToken {})) => GcpAuth::AccessToken {
					r#type: Some(auth::gcp::AccessToken),
					credential,
				},
				Some(gcp::TokenType::IdToken(gcp::IdToken { audience })) => GcpAuth::IdToken {
					r#type: auth::gcp::IdToken,
					audience,
					credential,
				},
			})
		},
		Some(proto::agent::backend_auth_policy::Kind::Aws(a)) => {
			let aws_auth = match a.kind {
				Some(proto::agent::aws::Kind::ExplicitConfig(config)) => AwsAuth::ExplicitConfig {
					access_key_id: config.access_key_id.into(),
					secret_access_key: config.secret_access_key.into(),
					region: if config.region.is_empty() {
						None
					} else {
						Some(config.region.clone())
					},
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
						Some(azure_explicit_config::CredentialSource::ClientSecret(cs)) => {
							auth::azure::AzureAuthCredentialSource::ClientSecret {
								tenant_id: cs.tenant_id,
								client_id: cs.client_id,
								client_secret: cs.client_secret.into(),
							}
						},
						Some(azure_explicit_config::CredentialSource::ManagedIdentityCredential(mic)) => {
							auth::azure::AzureAuthCredentialSource::ManagedIdentity {
								user_assigned_identity: mic.user_assigned_identity.and_then(|uami| {
									uami.id.map(|id| match id {
										user_assigned_identity::Id::ClientId(c) => {
											auth::azure::AzureUserAssignedIdentity::ClientId(c)
										},
										user_assigned_identity::Id::ObjectId(o) => {
											auth::azure::AzureUserAssignedIdentity::ObjectId(o)
										},
										user_assigned_identity::Id::ResourceId(r) => {
											auth::azure::AzureUserAssignedIdentity::ResourceId(r)
										},
									})
								}),
							}
						},
						Some(azure_explicit_config::CredentialSource::WorkloadIdentityCredential(_)) => {
							auth::azure::AzureAuthCredentialSource::WorkloadIdentity {}
						},
						None => {
							return Err(ProtoError::MissingRequiredField);
						},
					};
					auth::azure::AzureAuth::ExplicitConfig {
						credential_source: src,
						cached_cred: Default::default(),
					}
				},
				Some(proto::agent::azure::Kind::DeveloperImplicit(_)) => {
					auth::azure::AzureAuth::DeveloperImplicit {
						cached_cred: Default::default(),
					}
				},
				Some(proto::agent::azure::Kind::Implicit(_)) => auth::azure::AzureAuth::Implicit {
					cached_cred: Default::default(),
				},
				None => return Err(ProtoError::MissingRequiredField),
			};
			BackendAuth::Azure(azure_auth)
		},
		None => return Err(ProtoError::MissingRequiredField),
	})
}

fn listener_protocol_from_proto(
	protocol: proto::agent::Protocol,
	tls: Option<&proto::agent::TlsConfig>,
	diagnostics: &mut Diagnostics,
) -> Result<ListenerProtocol, ProtoError> {
	use crate::types::proto::agent::Protocol;
	match (protocol, tls) {
		(Protocol::Unknown, _) => Err(ProtoError::EnumParse("unknown protocol".into())),
		(Protocol::Http, None) => Ok(ListenerProtocol::HTTP),
		(Protocol::Https, Some(tls)) => Ok(ListenerProtocol::HTTPS(server_tls_config_from_proto(
			tls,
			diagnostics,
		))),
		// TLS termination
		(Protocol::Tls, Some(tls)) => Ok(ListenerProtocol::TLS(Some(server_tls_config_from_proto(
			tls,
			diagnostics,
		)))),
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

impl Bind {
	pub fn from_xds(
		s: &proto::agent::Bind,
		ipv6_enabled: bool,
		_diagnostics: &mut Diagnostics,
	) -> Result<Self, ProtoError> {
		let address = if cfg!(target_family = "unix") && ipv6_enabled {
			SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), s.port as u16)
		} else {
			SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), s.port as u16)
		};
		Ok(Self {
			key: s.key.clone().into(),
			address,
			listeners: Default::default(),
			protocol: match proto::agent::bind::Protocol::try_from(s.protocol)? {
				proto::agent::bind::Protocol::Http => BindProtocol::http,
				proto::agent::bind::Protocol::Tcp => BindProtocol::tcp,
				proto::agent::bind::Protocol::Tls => BindProtocol::tls,
			},
			tunnel_protocol: match proto::agent::bind::TunnelProtocol::try_from(s.tunnel_protocol)? {
				proto::agent::bind::TunnelProtocol::Direct => TunnelProtocol::Direct,
				proto::agent::bind::TunnelProtocol::HboneGateway => TunnelProtocol::HboneGateway,
				proto::agent::bind::TunnelProtocol::HboneWaypoint => TunnelProtocol::HboneWaypoint,
				proto::agent::bind::TunnelProtocol::Proxy => TunnelProtocol::Proxy,
			},
		})
	}
}

impl Listener {
	pub fn from_xds(
		s: &proto::agent::Listener,
		diagnostics: &mut Diagnostics,
	) -> Result<(Self, BindKey), ProtoError> {
		let proto = proto::agent::Protocol::try_from(s.protocol)?;
		let protocol = listener_protocol_from_proto(proto, s.tls.as_ref(), diagnostics)
			.map_err(|e| ProtoError::Generic(format!("{e}")))?;
		let l = Listener {
			key: strng::new(&s.key),
			name: s
				.name
				.as_ref()
				.ok_or(ProtoError::MissingRequiredField)?
				.into(),
			hostname: s.hostname.clone().into(),
			protocol,
		};
		Ok((l, strng::new(&s.bind_key)))
	}
}

/// Convert a proto NamespacedHostname to the Rust type.
fn service_key_from_proto(
	sk: Option<&proto::workload::NamespacedHostname>,
) -> Option<NamespacedHostname> {
	sk.filter(|sk| !sk.namespace.is_empty() || !sk.hostname.is_empty())
		.map(|sk| NamespacedHostname {
			namespace: Strng::from(&sk.namespace),
			hostname: Strng::from(&sk.hostname),
		})
}

impl TCPRoute {
	pub fn from_xds(
		s: &proto::agent::TcpRoute,
		_diagnostics: &mut Diagnostics,
	) -> Result<(Self, ListenerKey), ProtoError> {
		let r = TCPRoute {
			key: strng::new(&s.key),
			service_key: service_key_from_proto(s.service_key.as_ref()),
			name: s
				.name
				.as_ref()
				.ok_or(ProtoError::MissingRequiredField)?
				.into(),
			hostnames: s.hostnames.iter().map(strng::new).collect(),
			backends: s
				.backends
				.iter()
				.map(|b| TCPRouteBackendReference {
					weight: b.weight as usize,
					backend: resolve_simple_reference(b.backend.as_ref()),
					inline_policies: Vec::new(),
				})
				.collect::<Vec<_>>(),
		};
		Ok((r, strng::new(&s.listener_key)))
	}
}

impl Route {
	pub(crate) fn from_xds(
		s: &proto::agent::Route,
		diagnostics: &mut Diagnostics,
		oidc_cookie_encoder: Option<&OidcCookieEncoder>,
	) -> Result<(Self, ListenerKey, Option<RouteGroupKey>), ProtoError> {
		let name: RouteName = s
			.name
			.as_ref()
			.ok_or(ProtoError::MissingRequiredField)?
			.into();
		let inline_policies = s
			.traffic_policies
			.iter()
			.map(|policy| xds_traffic_policy_from_proto(policy, diagnostics, oidc_cookie_encoder))
			.collect::<Result<Vec<_>, _>>()?;
		let r = Route {
			key: strng::new(&s.key),
			service_key: service_key_from_proto(s.service_key.as_ref()),
			name,
			hostnames: s.hostnames.iter().map(strng::new).collect(),
			matches: s
				.matches
				.iter()
				.map(|m| route_match_from_proto(m, diagnostics))
				.collect::<Result<Vec<_>, _>>()?,
			backends: s
				.backends
				.iter()
				.map(|backend| route_backend_reference_from_proto(backend, diagnostics))
				.collect::<Result<Vec<_>, _>>()?,
			inline_policies,
		};
		Ok((
			r,
			strng::new(&s.listener_key),
			s.route_group_key
				.as_ref()
				.filter(|k| !k.is_empty())
				.map(strng::new),
		))
	}
}

pub(crate) fn backend_with_policies_from_proto(
	s: &proto::agent::Backend,
	diagnostics: &mut Diagnostics,
) -> Result<BackendWithPolicies, ProtoError> {
	use proto::agent::ai_backend::provider;
	use proto::agent::backend;
	let pols = s
		.inline_policies
		.iter()
		.map(|spec| backend_policy_from_proto(spec, diagnostics))
		.collect::<Result<Vec<_>, _>>()?;
	let name = s.name.as_ref().ok_or(ProtoError::MissingRequiredField)?;
	let backend = match &s.kind {
		Some(backend::Kind::Static(s)) => {
			let target = if !s.unix_path.is_empty() {
				Target::UnixSocket(std::path::PathBuf::from(&s.unix_path))
			} else {
				Target::from((s.host.as_str(), s.port as u16))
			};
			Backend::Opaque(name.into(), target)
		},
		Some(backend::Kind::Dynamic(_)) => Backend::Dynamic(name.into(), ()),
		Some(backend::Kind::Aws(a)) => {
			let aws_config = match &a.service {
				Some(proto::agent::aws_backend::Service::AgentCore(ac)) => {
					let agentcore_cfg = crate::agentcore::AgentCoreConfig::new(
						ac.agent_runtime_arn.clone(),
						ac.qualifier.clone(),
					)
					.map_err(|e| ProtoError::Generic(e.to_string()))?;
					crate::aws::AwsBackendConfig {
						service: crate::aws::AwsService::AgentCore(agentcore_cfg),
					}
				},
				None => {
					return Err(ProtoError::Generic(
						"AwsBackend: missing service".to_string(),
					));
				},
			};
			Backend::Aws(name.into(), aws_config)
		},
		Some(backend::Kind::Ai(a)) => {
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
						.map(|policy| backend_policy_from_proto(policy, diagnostics))
						.collect::<Result<Vec<_>, _>>()?;
					let provider = match &provider_config.provider {
						Some(provider::Provider::Openai(openai)) => AIProvider::OpenAI(llm::openai::Provider {
							model: openai.model.as_deref().map(strng::new),
						}),
						Some(provider::Provider::Gemini(gemini)) => AIProvider::Gemini(llm::gemini::Provider {
							model: gemini.model.as_deref().map(strng::new),
						}),
						Some(provider::Provider::Vertex(vertex)) => AIProvider::Vertex(llm::vertex::Provider {
							model: vertex.model.as_deref().map(strng::new),
							region: (!vertex.region.is_empty()).then(|| strng::new(&vertex.region)),
							project_id: strng::new(&vertex.project_id),
						}),
						Some(provider::Provider::Anthropic(anthropic)) => {
							AIProvider::Anthropic(llm::anthropic::Provider {
								model: anthropic.model.as_deref().map(strng::new),
							})
						},
						Some(provider::Provider::Bedrock(bedrock)) => {
							AIProvider::Bedrock(llm::bedrock::Provider {
								model: bedrock.model.as_deref().map(strng::new),
								region: strng::new(&bedrock.region),
								guardrail_identifier: bedrock.guardrail_identifier.as_deref().map(strng::new),
								guardrail_version: bedrock.guardrail_version.as_deref().map(strng::new),
							})
						},
						Some(provider::Provider::Azure(azure)) => {
							let resource_type = match azure.resource_type() {
								proto::agent::ai_backend::AzureResourceType::Foundry => {
									llm::azure::AzureResourceType::Foundry
								},
								_ => llm::azure::AzureResourceType::OpenAI,
							};
							AIProvider::Azure(llm::azure::Provider {
								model: azure.model.as_deref().map(strng::new),
								resource_name: strng::new(&azure.resource_name),
								resource_type,
								api_version: azure.api_version.as_deref().map(strng::new),
								project_name: azure.project_name.as_deref().map(strng::new),
								cached_cred: Default::default(),
							})
						},
						Some(provider::Provider::Azureopenai(_)) => {
							return Err(ProtoError::Generic(format!(
								"AI backend provider at index {provider_idx} uses deprecated azureOpenAI format; use azure instead"
							)));
						},
						None => {
							return Err(ProtoError::Generic(format!(
								"AI backend provider at index {provider_idx} is required"
							)));
						},
					};

					let provider_name = if provider_config.name.is_empty() {
						strng::literal!("default")
					} else {
						strng::new(&provider_config.name)
					};

					let np = NamedAIProvider {
						name: provider_name.clone(),
						provider,
						tokenize: false,
						host_override: provider_config
							.r#host_override
							.as_ref()
							.map(|o| Target::from((o.host.as_str(), o.port as u16))),
						path_override: provider_config.path_override.as_ref().map(strng::new),
						path_prefix: provider_config.path_prefix.as_ref().map(strng::new),
						inline_policies: pols,
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
			Backend::AI(name.into(), AIBackend { providers: es })
		},
		Some(proto::agent::backend::Kind::Mcp(m)) => Backend::MCP(
			name.into(),
			McpBackend {
				targets: m
					.targets
					.iter()
					.map(|t| mcp_target_from_proto(t, diagnostics).map(Arc::new))
					.collect::<Result<Vec<_>, _>>()?,
				stateful: match m.stateful_mode() {
					proto::agent::mcp_backend::StatefulMode::Stateful => true,
					proto::agent::mcp_backend::StatefulMode::Stateless => false,
				},
				always_use_prefix: match m.prefix_mode() {
					proto::agent::mcp_backend::PrefixMode::Always => true,
					proto::agent::mcp_backend::PrefixMode::Conditional => false,
				},
				failure_mode: match m.failure_mode() {
					proto::agent::mcp_backend::FailureMode::FailOpen => FailureMode::FailOpen,
					proto::agent::mcp_backend::FailureMode::FailClosed => FailureMode::FailClosed,
				},
				session_idle_ttl: crate::mcp::DEFAULT_SESSION_IDLE_TTL,
			},
		),
		None => {
			return Err(ProtoError::Generic("unknown backend".to_string()));
		},
	};
	Ok(BackendWithPolicies {
		backend,
		inline_policies: pols,
	})
}

fn mcp_target_from_proto(
	s: &proto::agent::McpTarget,
	_diagnostics: &mut Diagnostics,
) -> Result<McpTarget, ProtoError> {
	let proto = proto::agent::mcp_target::Protocol::try_from(s.protocol)?;
	let backend = resolve_simple_reference(s.backend.as_ref());
	validate_mcp_target_name(&s.name).map_err(ProtoError::Generic)?;

	Ok(McpTarget {
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

fn route_match_from_proto(
	s: &proto::agent::RouteMatch,
	diagnostics: &mut Diagnostics,
) -> Result<RouteMatch, ProtoError> {
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
		}) => regex_or_warn_invalid(diagnostics, "route.path", r)
			.map(PathMatch::Regex)
			.unwrap_or(PathMatch::Invalid),
		Some(proto::agent::PathMatch { kind: None }) => {
			return Err(ProtoError::Generic("invalid path match".to_string()));
		},
	};
	let method = s.method.as_ref().map(|m| MethodMatch {
		method: strng::new(&m.exact),
	});
	let headers = match convert_header_match(diagnostics, "route.headers", &s.headers) {
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
				value: regex_or_warn_invalid(diagnostics, format!("route.queryParams.{}", h.name), e)
					.map(QueryValueMatch::Regex)
					.unwrap_or(QueryValueMatch::Invalid),
			}),
		})
		.collect::<Result<Vec<_>, _>>()?;
	Ok(RouteMatch {
		headers,
		path,
		method,
		query,
	})
}

fn default_as_none<T: Default + PartialEq>(i: T) -> Option<T> {
	if i == Default::default() {
		None
	} else {
		Some(i)
	}
}

fn authorization_from_proto(
	rbac: &proto::agent::traffic_policy_spec::Rbac,
	diagnostics: &mut Diagnostics,
) -> Authorization {
	// Convert allow rules
	let mut allow_exprs = Vec::new();
	for allow_rule in &rbac.allow {
		allow_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"traffic.authorization.allow",
			allow_rule,
		));
	}
	// Convert deny rules
	let mut deny_exprs = Vec::new();
	for deny_rule in &rbac.deny {
		deny_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"traffic.authorization.deny",
			deny_rule,
		));
	}

	let mut require_exprs = Vec::new();
	for require_rule in &rbac.require {
		require_exprs.push(permissive_cel_expression_arc(
			diagnostics,
			"traffic.authorization.require",
			require_rule,
		));
	}

	// Create PolicySet using the same pattern as in de_policies function
	let policy_set = authorization::PolicySet::new(allow_exprs, deny_exprs, require_exprs);
	Authorization(authorization::RuleSet::new(policy_set))
}

fn transformation_from_proto(
	spec: &proto::agent::traffic_policy_spec::TransformationPolicy,
	diagnostics: &mut Diagnostics,
) -> Result<Transformation, ProtoError> {
	fn convert_transform(
		t: &Option<proto::agent::traffic_policy_spec::transformation_policy::Transform>,
	) -> LocalTransform {
		let mut add = Vec::new();
		let mut set = Vec::new();
		let mut remove = Vec::new();
		let mut body = None;
		let mut metadata = Vec::new();

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
			for (k, v) in &t.metadata {
				metadata.push((k.clone().into(), v.clone().into()));
			}
		}

		LocalTransform {
			add,
			set,
			remove,
			body,
			metadata,
		}
	}

	let request = Some(convert_transform(&spec.request));
	let response = Some(convert_transform(&spec.response));
	let config = LocalTransformationConfig { request, response };
	Transformation::try_from_local_config_with_warnings(config, false, |expression, err| {
		diagnostics.add_warning(format!(
			"invalid CEL expression for transformation: {err}; replacing {expression:?} with an expression that always fails",
		));
	})
	.map_err(|e| ProtoError::Generic(e.to_string()))
}

fn backend_policy_from_proto(
	spec: &proto::agent::BackendPolicySpec,
	diagnostics: &mut Diagnostics,
) -> Result<BackendTrafficPolicy, ProtoError> {
	use crate::types::proto::agent::backend_policy_spec as bps;
	Ok(match &spec.kind {
		Some(bps::Kind::A2a(_)) => BackendTrafficPolicy::A2a(A2aPolicy {}),
		Some(bps::Kind::InferenceRouting(ir)) => {
			let failure_mode = match bps::inference_routing::FailureMode::try_from(ir.failure_mode)? {
				bps::inference_routing::FailureMode::Unknown
				| bps::inference_routing::FailureMode::FailClosed => http::ext_proc::FailureMode::FailClosed,
				bps::inference_routing::FailureMode::FailOpen => http::ext_proc::FailureMode::FailOpen,
			};
			BackendTrafficPolicy::InferenceRouting(http::ext_proc::InferenceRouting {
				target: Arc::new(resolve_simple_reference(ir.endpoint_picker.as_ref())),
				destination_mode: http::ext_proc::InferenceRoutingDestinationMode::Validated,
				failure_mode,
			})
		},
		Some(bps::Kind::BackendHttp(bhttp)) => {
			let ver = bps::backend_http::HttpVersion::try_from(bhttp.version)?;
			BackendTrafficPolicy::HTTP(backend::HTTP {
				version: match ver {
					HttpVersion::Unspecified => None,
					HttpVersion::Http1 => Some(::http::Version::HTTP_11),
					HttpVersion::Http2 => Some(::http::Version::HTTP_2),
				},
				request_timeout: bhttp.request_timeout.map(convert_duration),
			})
		},
		Some(bps::Kind::BackendTcp(btcp)) => BackendTrafficPolicy::TCP(backend::TCP {
			connect_timeout: btcp
				.connect_timeout
				.map(convert_duration)
				.unwrap_or(backend::defaults::connect_timeout()),
			keepalives: btcp
				.keepalive
				.as_ref()
				.map(types::agent::KeepaliveConfig::from)
				.unwrap_or_default(),
		}),
		Some(bps::Kind::BackendTunnel(bt)) => BackendTrafficPolicy::Tunnel(backend::Tunnel {
			proxy: Arc::new(resolve_simple_reference(bt.proxy.as_ref())),
		}),
		Some(bps::Kind::BackendTls(btls)) => {
			let mode = bps::backend_tls::VerificationMode::try_from(btls.verification)?;
			let tls = http::backendtls::ResolvedBackendTLS {
				cert: btls.cert.clone(),
				key: btls.key.clone(),
				root: btls.root.clone(),
				insecure: mode == bps::backend_tls::VerificationMode::InsecureAll,
				insecure_host: mode == bps::backend_tls::VerificationMode::InsecureHost,
				hostname: btls.hostname.clone(),
				alpn: btls.alpn.as_ref().map(|a| a.protocols.clone()),
				subject_alt_names: if btls.verify_subject_alt_names.is_empty() {
					None
				} else {
					Some(btls.verify_subject_alt_names.clone())
				},
				key_exchange_groups: convert_tls_key_exchange_groups(
					&btls.key_exchange_groups,
					diagnostics,
				),
			}
			.try_into()
			.map_err(|e| ProtoError::Generic(e.to_string()))?;
			BackendTrafficPolicy::BackendTLS(tls)
		},
		Some(bps::Kind::Auth(auth)) => {
			BackendTrafficPolicy::BackendAuth(backend_auth_from_proto(auth.clone(), diagnostics)?)
		},
		Some(bps::Kind::McpAuthorization(rbac)) => {
			BackendTrafficPolicy::McpAuthorization(mcp_authorization_from_proto(rbac, diagnostics))
		},
		Some(bps::Kind::McpAuthentication(ma)) => {
			BackendTrafficPolicy::McpAuthentication(mcp_authentication_from_proto(ma, diagnostics)?)
		},
		Some(bps::Kind::Ai(ai)) => {
			BackendTrafficPolicy::AI(Arc::new(convert_backend_ai_policy(ai, diagnostics)?))
		},
		Some(bps::Kind::Transformation(tp)) => {
			BackendTrafficPolicy::Transformation(Arc::new(transformation_from_proto(tp, diagnostics)?))
		},
		Some(bps::Kind::RequestHeaderModifier(rhm)) => {
			BackendTrafficPolicy::RequestHeaderModifier(http::filters::HeaderModifier {
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
			BackendTrafficPolicy::ResponseHeaderModifier(Arc::new(http::filters::HeaderModifier {
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
			}))
		},
		Some(bps::Kind::RequestRedirect(rr)) => {
			BackendTrafficPolicy::RequestRedirect(http::filters::RequestRedirect {
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
				.map(|m| http::filters::RequestMirror {
					backend: resolve_simple_reference(m.backend.as_ref()),
					percentage: m.percentage / 100.0,
				})
				.collect::<Vec<_>>();
			BackendTrafficPolicy::RequestMirror(mirrors)
		},
		Some(bps::Kind::Health(h)) => BackendTrafficPolicy::Health(convert_health(h, diagnostics)),
		None => return Err(ProtoError::MissingRequiredField),
	})
}

fn convert_health(
	h: &proto::agent::backend_policy_spec::Health,
	diagnostics: &mut Diagnostics,
) -> health::Policy {
	let unhealthy_expression = if h.unhealthy_condition.is_empty() {
		None
	} else {
		Some(permissive_cel_expression_arc(
			diagnostics,
			"backend.health.unhealthyCondition",
			&h.unhealthy_condition,
		))
	};
	let eviction = h.eviction.as_ref().map(|ev| health::Eviction {
		duration: ev.duration.map(convert_duration),
		restore_health: ev.restore_health,
		consecutive_failures: ev.consecutive_failures,
		health_threshold: ev.health_threshold,
	});
	health::Policy {
		unhealthy_expression,
		eviction,
	}
}

fn xds_phased_traffic_policy_from_proto(
	spec: &proto::agent::TrafficPolicySpec,
	diagnostics: &mut Diagnostics,
	oidc_cookie_encoder: Option<&OidcCookieEncoder>,
) -> Result<agent::PhasedTrafficPolicy, ProtoError> {
	let policy = xds_traffic_policy_from_proto(spec, diagnostics, oidc_cookie_encoder)?;
	Ok(agent::PhasedTrafficPolicy {
		phase: match proto::agent::traffic_policy_spec::PolicyPhase::try_from(spec.phase)? {
			proto::agent::traffic_policy_spec::PolicyPhase::Route => PolicyPhase::Route,
			proto::agent::traffic_policy_spec::PolicyPhase::Gateway => PolicyPhase::Gateway,
		},
		policy,
	})
}

fn xds_traffic_policy_from_proto(
	spec: &proto::agent::TrafficPolicySpec,
	diagnostics: &mut Diagnostics,
	oidc_cookie_encoder: Option<&OidcCookieEncoder>,
) -> Result<TrafficPolicy, ProtoError> {
	use crate::types::proto::agent::traffic_policy_spec as tps;

	match &spec.kind {
		Some(tps::Kind::Oidc(oidc)) => {
			// Check encoder before decode so we don't allocate client_secret on a rejected path.
			let encoder = oidc_cookie_encoder.ok_or_else(|| {
				ProtoError::Generic(
					"received xDS OIDC policy but SESSION_KEY is not configured on this gateway".into(),
				)
			})?;
			let policy = oidc_policy_from_proto(oidc)?;
			policy
				.compile(encoder)
				.map(|p| TrafficPolicy::Oidc(RequestPolicy::single(p)))
				.map_err(|e| ProtoError::Generic(format!("failed to compile xDS OIDC policy: {e}")))
		},
		_ => traffic_policy_from_proto(spec, diagnostics),
	}
}

fn traffic_policy_from_proto(
	spec: &proto::agent::TrafficPolicySpec,
	diagnostics: &mut Diagnostics,
) -> Result<TrafficPolicy, ProtoError> {
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
			// Yes, its single with a vec, because we originally supported multiple rate limit policies before
			// we added the generic multiple support.
			// If we end up adding "Multiple and execute all" to RequestPolicy, we could translate to that;
			// until this, this is a single policy with multiple rules.
			TrafficPolicy::LocalRateLimit(RequestPolicy::single(vec![
				spec
					.try_into()
					.map_err(|e| ProtoError::Generic(format!("invalid rate limit: {e}")))?,
			]))
		},
		Some(tps::Kind::ExtAuthz(ea)) => {
			use proto::agent::traffic_policy_spec::external_auth;
			let target = resolve_simple_reference(ea.target.as_ref());
			let failure_mode = match external_auth::FailureMode::try_from(ea.failure_mode) {
				Ok(external_auth::FailureMode::Allow) => http::ext_authz::FailureMode::Allow,
				Ok(external_auth::FailureMode::Deny) => http::ext_authz::FailureMode::Deny,
				Ok(external_auth::FailureMode::DenyWithStatus) => {
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
			let protocol = match ea
				.protocol
				.as_ref()
				.ok_or(ProtoError::MissingRequiredField)?
			{
				external_auth::Protocol::Grpc(g) => {
					let metadata: HashMap<_, _> = g
						.metadata
						.iter()
						.map(|(k, v)| {
							let ve = permissive_cel_expression_arc(
								diagnostics,
								format!("traffic.extAuthz.grpc.metadata.{k}"),
								v,
							);
							Ok::<_, ProtoError>((k.to_owned(), ve))
						})
						.collect::<Result<_, _>>()?;
					http::ext_authz::Protocol::Grpc {
						context: Some(g.context.clone()),
						metadata: if metadata.is_empty() {
							None
						} else {
							Some(metadata)
						},
					}
				},
				external_auth::Protocol::Http(h) => http::ext_authz::Protocol::Http {
					path: h.path.as_ref().map(|expr| {
						permissive_cel_expression_arc(diagnostics, "traffic.extAuthz.http.path", expr)
					}),
					redirect: h.redirect.as_ref().map(|expr| {
						permissive_cel_expression_arc(diagnostics, "traffic.extAuthz.http.redirect", expr)
					}),
					include_response_headers: h
						.include_response_headers
						.iter()
						.map(|k| HeaderName::try_from(k.as_str()))
						.collect::<Result<_, _>>()?,
					add_request_headers: h
						.add_request_headers
						.iter()
						.map(|(k, v)| {
							let tk = HeaderOrPseudo::try_from(k.as_str())?;
							let tv = permissive_cel_expression_arc(
								diagnostics,
								format!("traffic.extAuthz.http.addRequestHeaders.{k}"),
								v.as_str(),
							);
							Ok::<_, anyhow::Error>((tk, tv))
						})
						.collect::<Result<_, _>>()
						.map_err(|e| ProtoError::Generic(e.to_string()))?,
					metadata: h
						.metadata
						.iter()
						.map(|(k, v)| {
							let ve = permissive_cel_expression(
								diagnostics,
								format!("traffic.extAuthz.http.metadata.{k}"),
								v,
							);
							Ok::<_, ProtoError>((k.to_owned(), Arc::new(ve)))
						})
						.collect::<Result<_, _>>()?,
				},
			};
			TrafficPolicy::ExtAuthz(RequestPolicy::single(http::ext_authz::ExtAuthz {
				protocol,
				target: Arc::new(target),
				// Not supported inline from xDS
				policies: Vec::new(),
				failure_mode,
				include_request_headers: ea
					.include_request_headers
					.iter()
					.filter_map(
						|s| match crate::http::HeaderOrPseudo::try_from(s.as_str()) {
							Ok(h) => Some(h),
							Err(_) => {
								diagnostics.add_warning(format!(
									"invalid header in extauth include_request_headers; skipping: {s}"
								));
								None
							},
						},
					)
					.collect(),
				include_request_body,
			}))
		},
		Some(tps::Kind::Authorization(rbac)) => {
			TrafficPolicy::Authorization(authorization_from_proto(rbac, diagnostics))
		},
		Some(tps::Kind::Oidc(_)) => {
			return Err(ProtoError::Generic(
				"internal error: OIDC policy must be decoded through xds_traffic_policy_from_proto".into(),
			));
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
					let jwt_validation_options = p
						.jwt_validation_options
						.as_ref()
						.map(|vo| http::jwt::JWTValidationOptions {
							required_claims: vo.required_claims.iter().cloned().collect(),
						})
						.unwrap_or_default();
					http::jwt::Provider::from_jwks(
						jwk_set,
						p.issuer.clone(),
						audiences,
						jwt_validation_options,
					)
					.map_err(|e| ProtoError::Generic(format!("failed to create JWT config: {e}")))
				})
				.collect::<Result<Vec<_>, _>>()?;
			let jwt_auth = http::jwt::Jwt::from_providers(
				providers,
				mode,
				authorization_location(
					jwt.authorization_location.as_ref(),
					http::auth::AuthorizationLocation::bearer_header(),
				)?,
			);
			let mcp = match &jwt.mcp {
				Some(mcp) => {
					if jwt.providers.len() != 1 {
						return Err(ProtoError::Generic(format!(
							"JWT MCP extension requires exactly one provider, found {}",
							jwt.providers.len()
						)));
					}
					let provider = &jwt.providers[0];
					Some(build_mcp_authentication(
						provider.issuer.clone(),
						provider.audiences.clone(),
						mcp.provider,
						convert_mcp_resource_metadata(mcp.resource_metadata.as_ref().map(|rm| rm.extra.iter())),
						Arc::new(jwt_auth.clone()),
						match tps::jwt::Mode::try_from(jwt.mode)
							.map_err(|_| ProtoError::EnumParse("invalid JWT mode".to_string()))?
						{
							tps::jwt::Mode::Optional => McpAuthenticationMode::Optional,
							tps::jwt::Mode::Strict => McpAuthenticationMode::Strict,
							tps::jwt::Mode::Permissive => McpAuthenticationMode::Permissive,
						},
					))
				},
				None => None,
			};
			TrafficPolicy::JwtAuth(RequestPolicy::single(JwtAuthentication {
				jwt: jwt_auth,
				mcp,
			}))
		},
		Some(tps::Kind::Transformation(tp)) => TrafficPolicy::Transformation(RequestPolicy::single(
			transformation_from_proto(tp, diagnostics)?,
		)),
		Some(tps::Kind::RemoteRateLimit(rrl)) => {
			let descriptors = rrl
				.descriptors
				.iter()
				.map(
					|d| -> Result<http::remoteratelimit::DescriptorEntry, ProtoError> {
						let entries: Vec<_> = d
							.entries
							.iter()
							.map(|e| {
								http::remoteratelimit::Descriptor(
									e.key.clone(),
									permissive_cel_expression(
										diagnostics,
										format!("traffic.remoteRateLimit.descriptors.{}", e.key),
										e.value.clone(),
									),
								)
							})
							.collect();
						Ok(http::remoteratelimit::DescriptorEntry {
							entries: Arc::new(entries),
							limit_type: match tps::remote_rate_limit::Type::try_from(d.r#type)
								.unwrap_or(tps::remote_rate_limit::Type::Requests)
							{
								tps::remote_rate_limit::Type::Requests => {
									http::localratelimit::RateLimitType::Requests
								},
								tps::remote_rate_limit::Type::Tokens => http::localratelimit::RateLimitType::Tokens,
							},
							limit_override: d.limit_override.as_ref().map(|expr| {
								permissive_cel_expression_arc(
									diagnostics,
									"traffic.remoteRateLimit.limitOverride",
									expr,
								)
							}),
						})
					},
				)
				.collect::<Result<Vec<_>, _>>()?;
			let target = resolve_simple_reference(rrl.target.as_ref());
			let failure_mode = match tps::remote_rate_limit::FailureMode::try_from(rrl.failure_mode) {
				Ok(tps::remote_rate_limit::FailureMode::FailOpen) => {
					http::remoteratelimit::FailureMode::FailOpen
				},
				// Default to FailClosed (proto default is FAIL_CLOSED = 0)
				_ => http::remoteratelimit::FailureMode::FailClosed,
			};
			TrafficPolicy::RemoteRateLimit(RequestPolicy::single(
				http::remoteratelimit::RemoteRateLimit {
					domain: rrl.domain.clone(),
					target: Arc::new(target),
					// Not supported inline from xDS
					policies: Vec::new(),
					descriptors: Arc::new(http::remoteratelimit::DescriptorSet(descriptors)),
					failure_mode,
				},
			))
		},
		Some(tps::Kind::Csrf(csrf_spec)) => {
			let additional_origins: std::collections::HashSet<String> =
				csrf_spec.additional_origins.iter().cloned().collect();
			TrafficPolicy::Csrf(RequestPolicy::single(crate::http::csrf::Csrf::new(
				additional_origins,
			)))
		},
		Some(tps::Kind::ExtProc(ep)) => {
			let target = resolve_simple_reference(ep.target.as_ref());
			let failure_mode = match tps::ext_proc::FailureMode::try_from(ep.failure_mode) {
				Ok(tps::ext_proc::FailureMode::FailOpen) => http::ext_proc::FailureMode::FailOpen,
				_ => http::ext_proc::FailureMode::FailClosed,
			};
			fn to_cel_attrs(
				diagnostics: &mut Diagnostics,
				context: &str,
				attrs: &HashMap<String, String>,
			) -> Option<HashMap<String, Arc<cel::Expression>>> {
				if attrs.is_empty() {
					None
				} else {
					Some(
						attrs
							.iter()
							.map(|(k, v)| {
								(
									k.clone(),
									permissive_cel_expression_arc(diagnostics, format!("{context}.{k}"), v),
								)
							})
							.collect(),
					)
				}
			}
			TrafficPolicy::ExtProc(RequestPolicy::single(http::ext_proc::ExtProc {
				target: Arc::new(target),
				// Not supported inline from xDS
				policies: Vec::new(),
				failure_mode,
				request_attributes: to_cel_attrs(
					diagnostics,
					"traffic.extProc.requestAttributes",
					&ep.request_attributes,
				),
				response_attributes: to_cel_attrs(
					diagnostics,
					"traffic.extProc.responseAttributes",
					&ep.response_attributes,
				),
				metadata_context: if ep.metadata_context.is_empty() {
					None
				} else {
					Some(
						ep.metadata_context
							.iter()
							.fold(HashMap::new(), |mut meta, (namespace, data)| {
								meta.insert(
									namespace.to_string(),
									data
										.context
										.iter()
										.map(|(k, v)| {
											(
												k.clone(),
												permissive_cel_expression_arc(
													diagnostics,
													format!("traffic.extProc.metadataContext.{namespace}.{k}"),
													v,
												),
											)
										})
										.collect(),
								);
								meta
							}),
					)
				},
			}))
		},
		Some(tps::Kind::RequestHeaderModifier(rhm)) => {
			TrafficPolicy::RequestHeaderModifier(RequestPolicy::single(http::filters::HeaderModifier {
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
			}))
		},
		Some(tps::Kind::ResponseHeaderModifier(rhm)) => {
			TrafficPolicy::ResponseHeaderModifier(Arc::new(http::filters::HeaderModifier {
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
			}))
		},
		Some(tps::Kind::RequestRedirect(rr)) => {
			TrafficPolicy::RequestRedirect(RequestPolicy::single(http::filters::RequestRedirect {
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
			}))
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
			TrafficPolicy::UrlRewrite(RequestPolicy::single(http::filters::UrlRewrite {
				authority,
				path,
			}))
		},
		Some(tps::Kind::RequestMirror(m)) => {
			let mirrors = m
				.mirrors
				.iter()
				.map(|m| http::filters::RequestMirror {
					backend: resolve_simple_reference(m.backend.as_ref()),
					percentage: m.percentage / 100.0,
				})
				.collect::<Vec<_>>();
			TrafficPolicy::RequestMirror(mirrors)
		},
		Some(tps::Kind::DirectResponse(dr)) => {
			TrafficPolicy::DirectResponse(RequestPolicy::single(http::filters::DirectResponse {
				body: bytes::Bytes::copy_from_slice(&dr.body),
				status: StatusCode::from_u16(dr.status as u16)?,
			}))
		},
		Some(tps::Kind::Cors(c)) => TrafficPolicy::CORS(RequestPolicy::single(
			http::cors::Cors::try_from(http::cors::CorsSerde {
				allow_credentials: c.allow_credentials,
				allow_headers: c.allow_headers.clone(),
				allow_methods: c.allow_methods.clone(),
				allow_origins: c.allow_origins.clone(),
				expose_headers: c.expose_headers.clone(),
				max_age: c.max_age.as_ref().map(|d| (*d).try_into()).transpose()?,
			})
			.map_err(|e| ProtoError::Generic(e.to_string()))?,
		)),
		Some(tps::Kind::BasicAuth(ba)) => {
			let mode = match tps::basic_authentication::Mode::try_from(ba.mode)
				.map_err(|_| ProtoError::EnumParse("invalid Basic Auth mode".to_string()))?
			{
				tps::basic_authentication::Mode::Strict => http::basicauth::Mode::Strict,
				tps::basic_authentication::Mode::Optional => http::basicauth::Mode::Optional,
			};
			TrafficPolicy::BasicAuth(RequestPolicy::single(
				http::basicauth::BasicAuthentication::new(
					&ba.htpasswd_content,
					ba.realm.clone(),
					mode,
					authorization_location(
						ba.authorization_location.as_ref(),
						http::auth::AuthorizationLocation::basic_header(),
					)?,
				),
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
			TrafficPolicy::APIKey(RequestPolicy::single(
				http::apikey::APIKeyAuthentication::new(
					keys,
					mode,
					authorization_location(
						ba.authorization_location.as_ref(),
						http::auth::AuthorizationLocation::bearer_header(),
					)?,
				),
			))
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

fn convert_duration(d: prost_types::Duration) -> Duration {
	Duration::from_secs(d.seconds as u64) + Duration::from_nanos(d.nanos as u64)
}

fn oidc_policy_from_proto(
	oidc: &proto::agent::traffic_policy_spec::Oidc,
) -> Result<crate::http::oidc::PreparedOidcPolicy, ProtoError> {
	use std::str::FromStr;

	use proto::agent::traffic_policy_spec::oidc::TokenEndpointAuth as ProtoAuth;

	use crate::http::oidc::{
		ClientCredentials, PolicyId, ProviderEndpoint, RedirectUri, resolve_oidc_policy_from_xds,
	};

	let policy_id = PolicyId::from_str(&oidc.policy_id)
		.map_err(|e| ProtoError::Generic(format!("xDS OIDC: invalid policy_id: {e}")))?;
	let authorization_endpoint = ProviderEndpoint::try_from(oidc.authorization_endpoint.as_str())
		.map_err(|e| ProtoError::Generic(format!("xDS OIDC: invalid authorization_endpoint: {e}")))?;
	let token_endpoint = ProviderEndpoint::try_from(oidc.token_endpoint.as_str())
		.map_err(|e| ProtoError::Generic(format!("xDS OIDC: invalid token_endpoint: {e}")))?;
	let redirect_uri = RedirectUri::parse(oidc.redirect_uri.clone())
		.map_err(|e| ProtoError::Generic(format!("xDS OIDC: invalid redirect_uri: {e}")))?;
	let token_endpoint_auth = match ProtoAuth::try_from(oidc.token_endpoint_auth).map_err(|_| {
		ProtoError::EnumParse(format!(
			"xDS OIDC: invalid token_endpoint_auth value {}",
			oidc.token_endpoint_auth
		))
	})? {
		ProtoAuth::Unspecified => {
			return Err(ProtoError::Generic(
				"xDS OIDC: token_endpoint_auth must not be unspecified".into(),
			));
		},
		ProtoAuth::ClientSecretBasic => http::oauth::TokenEndpointAuth::ClientSecretBasic,
		ProtoAuth::ClientSecretPost => http::oauth::TokenEndpointAuth::ClientSecretPost,
		ProtoAuth::None => http::oauth::TokenEndpointAuth::Public,
	};
	if oidc.issuer.is_empty() {
		return Err(ProtoError::Generic(
			"xDS OIDC: issuer must not be empty".into(),
		));
	}
	if oidc.client_id.is_empty() {
		return Err(ProtoError::Generic(
			"xDS OIDC: client_id must not be empty".into(),
		));
	}
	if oidc.jwks_inline.is_empty() {
		return Err(ProtoError::Generic(
			"xDS OIDC: jwks_inline must not be empty".into(),
		));
	}

	// Build credentials directly with proto-named errors so the xDS path doesn't
	// re-validate the same pairing inside `ClientCredentials::from_parts`.
	let credentials = match (token_endpoint_auth, oidc.client_secret.is_empty()) {
		(http::oauth::TokenEndpointAuth::Public, true) => ClientCredentials::Public,
		(http::oauth::TokenEndpointAuth::Public, false) => {
			return Err(ProtoError::Generic(
				"xDS OIDC: token_endpoint_auth=None (public client) must not carry a client_secret over xDS".into(),
			));
		},
		(method, true) => {
			return Err(ProtoError::Generic(format!(
				"xDS OIDC: token_endpoint_auth={} requires a non-empty client_secret",
				method.as_str()
			)));
		},
		(http::oauth::TokenEndpointAuth::ClientSecretBasic, false) => {
			ClientCredentials::ClientSecretBasic {
				client_secret: secrecy::SecretString::new(oidc.client_secret.clone().into()),
			}
		},
		(http::oauth::TokenEndpointAuth::ClientSecretPost, false) => {
			ClientCredentials::ClientSecretPost {
				client_secret: secrecy::SecretString::new(oidc.client_secret.clone().into()),
			}
		},
	};

	resolve_oidc_policy_from_xds(
		policy_id,
		oidc.issuer.clone(),
		authorization_endpoint,
		token_endpoint,
		&oidc.jwks_inline,
		oidc.client_id.clone(),
		credentials,
		redirect_uri,
		oidc.scopes.clone(),
	)
	.map_err(|e| ProtoError::Generic(format!("failed to decode xDS OIDC policy: {e}")))
}

fn authorization_location(
	location: Option<&proto::agent::AuthorizationLocation>,
	default: http::auth::AuthorizationLocation,
) -> Result<http::auth::AuthorizationLocation, ProtoError> {
	use proto::agent::authorization_location::Kind;

	let Some(location) = location else {
		return Ok(default);
	};

	match location.kind.as_ref() {
		Some(Kind::Header(header)) => Ok(http::auth::AuthorizationLocation::Header {
			name: header.name.parse()?,
			prefix: header.prefix.clone().map(Into::into),
		}),
		Some(Kind::QueryParameter(query)) => Ok(http::auth::AuthorizationLocation::QueryParameter {
			name: query.name.clone().into(),
		}),
		Some(Kind::Cookie(cookie)) => Ok(http::auth::AuthorizationLocation::Cookie {
			name: cookie.name.clone().into(),
		}),
		None => Ok(default),
	}
}

/// Like [`authorization_location`], but returns `None` when the proto field is absent,
/// preserving the distinction between "not set" (default) and "explicitly configured".
fn optional_authorization_location(
	location: Option<&proto::agent::AuthorizationLocation>,
) -> Result<Option<http::auth::AuthorizationLocation>, ProtoError> {
	use proto::agent::authorization_location::Kind;

	let Some(location) = location else {
		return Ok(None);
	};

	match location.kind.as_ref() {
		Some(Kind::Header(header)) => Ok(Some(http::auth::AuthorizationLocation::Header {
			name: header.name.parse()?,
			prefix: header.prefix.clone().map(Into::into),
		})),
		Some(Kind::QueryParameter(query)) => {
			Ok(Some(http::auth::AuthorizationLocation::QueryParameter {
				name: query.name.clone().into(),
			}))
		},
		Some(Kind::Cookie(cookie)) => Ok(Some(http::auth::AuthorizationLocation::Cookie {
			name: cookie.name.clone().into(),
		})),
		None => Ok(None),
	}
}

fn frontend_policy_from_proto(
	spec: &proto::agent::FrontendPolicySpec,
	diagnostics: &mut Diagnostics,
) -> Result<FrontendPolicy, ProtoError> {
	use crate::types::frontend;
	use crate::types::proto::agent::frontend_policy_spec as fps;

	let map_tls_version = |raw: Option<i32>| {
		raw.and_then(
			|raw| match proto::agent::tls_config::TlsVersion::try_from(raw).ok() {
				Some(proto::agent::tls_config::TlsVersion::TlsV12) => Some(frontend::TLSVersion::TLS_V1_2),
				Some(proto::agent::tls_config::TlsVersion::TlsV13) => Some(frontend::TLSVersion::TLS_V1_3),
				_ => None,
			},
		)
	};

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
			max_connection_duration: h.max_connection_duration.map(convert_duration),
		}),
		Some(fps::Kind::Tls(t)) => FrontendPolicy::TLS(frontend::TLS {
			handshake_timeout: t
				.handshake_timeout
				.map(convert_duration)
				.unwrap_or_else(crate::defaults::tls_handshake_timeout),
			alpn: t
				.alpn
				.as_ref()
				.map(|t| t.protocols.iter().map(|s| s.as_bytes().to_vec()).collect()),
			min_version: map_tls_version(t.min_version),
			max_version: map_tls_version(t.max_version),
			cipher_suites: convert_tls_cipher_suites(&t.cipher_suites, diagnostics),
			key_exchange_groups: convert_tls_key_exchange_groups(&t.key_exchange_groups, diagnostics),
		}),
		Some(fps::Kind::Tcp(t)) => FrontendPolicy::TCP(frontend::TCP {
			keepalives: t
				.keepalives
				.as_ref()
				.map(types::agent::KeepaliveConfig::from)
				.unwrap_or_default(),
		}),
		Some(fps::Kind::NetworkAuthorization(rbac)) => {
			let mut allow_exprs = Vec::new();
			for allow_rule in &rbac.allow {
				allow_exprs.push(permissive_cel_expression_arc(
					diagnostics,
					"frontend.networkAuthorization.allow",
					allow_rule,
				));
			}

			let mut deny_exprs = Vec::new();
			for deny_rule in &rbac.deny {
				deny_exprs.push(permissive_cel_expression_arc(
					diagnostics,
					"frontend.networkAuthorization.deny",
					deny_rule,
				));
			}

			let mut require_exprs = Vec::new();
			for require_rule in &rbac.require {
				require_exprs.push(permissive_cel_expression_arc(
					diagnostics,
					"frontend.networkAuthorization.require",
					require_rule,
				));
			}

			let policy_set = authorization::PolicySet::new(allow_exprs, deny_exprs, require_exprs);
			FrontendPolicy::NetworkAuthorization(frontend::NetworkAuthorization(
				authorization::RuleSet::new(policy_set),
			))
		},
		Some(fps::Kind::ProxyProtocol(p)) => {
			let version =
				match crate::types::proto::agent::frontend_policy_spec::proxy_protocol::Version::try_from(
					p.version,
				) {
					Ok(crate::types::proto::agent::frontend_policy_spec::proxy_protocol::Version::V1) => {
						frontend::ProxyVersion::V1
					},
					Ok(crate::types::proto::agent::frontend_policy_spec::proxy_protocol::Version::All) => {
						frontend::ProxyVersion::All
					},
					_ => frontend::ProxyVersion::V2,
				};
			let mode =
				match crate::types::proto::agent::frontend_policy_spec::proxy_protocol::Mode::try_from(
					p.mode,
				) {
					Ok(crate::types::proto::agent::frontend_policy_spec::proxy_protocol::Mode::Optional) => {
						frontend::ProxyMode::Optional
					},
					_ => frontend::ProxyMode::Strict,
				};
			FrontendPolicy::Proxy(frontend::Proxy { version, mode })
		},
		Some(fps::Kind::Logging(p)) => {
			let (add, rm) = p
				.fields
				.as_ref()
				.map(|f| {
					let add = f
						.add
						.iter()
						.map(|f| {
							let expr = permissive_cel_expression_arc(
								diagnostics,
								format!("frontend.logging.fields.add.{}", f.name),
								&f.expression,
							);
							Ok::<_, ProtoError>((f.name.clone(), expr))
						})
						.collect::<Result<Vec<_>, _>>()?;
					let rm = f.remove.clone();
					Ok::<_, ProtoError>((OrderedStringMap::from_iter(add), rm))
				})
				.transpose()?
				.unwrap_or_default();
			let otlp = p
				.otlp_access_log
				.as_ref()
				.map(|oal| -> Result<frontend::OtlpLoggingConfig, ProtoError> {
					let provider_backend = resolve_simple_reference(oal.provider_backend.as_ref());
					let policies = oal
						.inline_policies
						.iter()
						.map(|policy| backend_policy_from_proto(policy, diagnostics))
						.collect::<Result<Vec<_>, _>>()?;
					let protocol = match fps::logging::otlp_access_log::Protocol::try_from(oal.protocol) {
						Ok(fps::logging::otlp_access_log::Protocol::Grpc) => {
							types::agent::TracingProtocol::Grpc
						},
						_ => types::agent::TracingProtocol::Http,
					};
					let path = oal.path.clone().unwrap_or_else(|| "/v1/logs".to_string());
					Ok(frontend::OtlpLoggingConfig {
						provider_backend,
						policies,
						protocol,
						path,
					})
				})
				.transpose()?;
			let mut logging_policy = frontend::LoggingPolicy {
				filter: p
					.filter
					.as_ref()
					.map(|expr| permissive_cel_expression_arc(diagnostics, "frontend.logging.filter", expr)),
				add: Arc::new(add),
				remove: Arc::new(FzHashSet::new(rm)),
				otlp,
				access_log_policy: None,
			};
			logging_policy.init_access_log_policy();
			FrontendPolicy::AccessLog(logging_policy)
		},
		Some(fps::Kind::Tracing(t)) => {
			// Convert protobuf to TracingConfig
			let tracing_config = tracing_config_from_proto(t, diagnostics);

			// Prepare LoggingFields with the CEL attributes from TracingConfig
			let logging_fields = Arc::new(crate::telemetry::log::LoggingFields {
				remove: Arc::new(tracing_config.remove.iter().cloned().collect()),
				add: Arc::new(tracing_config.attributes.clone()),
			});

			FrontendPolicy::Tracing(Arc::new(types::agent::TracingPolicy {
				config: tracing_config,
				fields: logging_fields,
				tracer: once_cell::sync::OnceCell::new(),
			}))
		},
		Some(fps::Kind::Metrics(m)) => {
			let add = m
				.fields
				.as_ref()
				.map(|f| {
					f.add
						.iter()
						.map(|field| {
							let expr = permissive_cel_expression_arc(
								diagnostics,
								format!("frontend.metrics.fields.add.{}", field.name),
								&field.expression,
							);
							Ok::<_, ProtoError>((field.name.clone(), expr))
						})
						.collect::<Result<Vec<_>, _>>()
						.map(OrderedStringMap::from_iter)
				})
				.transpose()?
				.unwrap_or_default();
			FrontendPolicy::Metrics(frontend::MetricsFieldsPolicy { add: Arc::new(add) })
		},
		None => return Err(ProtoError::MissingRequiredField),
	})
}

fn tracing_config_from_proto(
	t: &proto::agent::frontend_policy_spec::Tracing,
	diagnostics: &mut Diagnostics,
) -> types::agent::TracingConfig {
	let provider_backend = resolve_simple_reference(t.provider_backend.as_ref());

	let attributes: OrderedStringMap<Arc<cel::Expression>> = t
		.attributes
		.iter()
		.map(|a| {
			(
				a.name.clone(),
				permissive_cel_expression_arc(
					diagnostics,
					format!("frontend.tracing.attributes.{}", a.name),
					&a.value,
				),
			)
		})
		.collect();

	let resources: OrderedStringMap<Arc<cel::Expression>> = t
		.resources
		.iter()
		.map(|a| {
			(
				a.name.clone(),
				permissive_cel_expression_arc(
					diagnostics,
					format!("frontend.tracing.resources.{}", a.name),
					&a.value,
				),
			)
		})
		.collect();

	// Optional per-policy sampling overrides
	let random_sampling = t
		.random_sampling
		.as_ref()
		.map(|s| permissive_cel_expression_arc(diagnostics, "frontend.tracing.randomSampling", s));
	let client_sampling = t
		.client_sampling
		.as_ref()
		.map(|s| permissive_cel_expression_arc(diagnostics, "frontend.tracing.clientSampling", s));

	let path = t.path.clone().unwrap_or_else(|| "/v1/traces".to_string());

	let protocol =
		match crate::types::proto::agent::frontend_policy_spec::tracing::Protocol::try_from(t.protocol)
		{
			Ok(crate::types::proto::agent::frontend_policy_spec::tracing::Protocol::Grpc) => {
				types::agent::TracingProtocol::Grpc
			},
			_ => types::agent::TracingProtocol::Http,
		};

	types::agent::TracingConfig {
		provider_backend,
		// Not supported inline from xDS
		policies: Vec::new(),
		attributes,
		resources,
		remove: t.remove.clone(),
		random_sampling,
		client_sampling,
		path,
		protocol,
	}
}

impl From<&proto::agent::KeepaliveConfig> for KeepaliveConfig {
	fn from(k: &proto::agent::KeepaliveConfig) -> Self {
		KeepaliveConfig {
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
		}
	}
}

fn policy_target_from_proto(t: &proto::agent::PolicyTarget) -> Result<PolicyTarget, ProtoError> {
	use crate::types::proto::agent::policy_target as tgt;
	match t.kind.as_ref() {
		Some(tgt::Kind::Gateway(g)) => Ok(PolicyTarget::Gateway(ListenerTarget {
			gateway_name: strng::new(&g.name),
			gateway_namespace: strng::new(&g.namespace),
			listener_name: g.listener.as_ref().map(Into::into),
			port: g
				.port
				.map(|p| {
					u16::try_from(p)
						.map_err(|_| ProtoError::Generic(format!("gateway target port out of range: {p}")))
				})
				.transpose()?,
		})),
		Some(tgt::Kind::Route(r)) => Ok(PolicyTarget::Route(RouteTarget {
			name: strng::new(&r.name),
			namespace: strng::new(&r.namespace),
			rule_name: r.route_rule.as_ref().map(Into::into),
			kind: (!r.kind.is_empty()).then(|| strng::new(&r.kind)),
		})),
		Some(tgt::Kind::Backend(b)) => Ok(PolicyTarget::Backend(BackendTarget::Backend {
			name: strng::new(&b.name),
			namespace: strng::new(&b.namespace),
			section: b.section.as_ref().map(Into::into),
		})),
		Some(tgt::Kind::Service(s)) => Ok(PolicyTarget::Backend(BackendTarget::Service {
			hostname: strng::new(&s.hostname),
			namespace: strng::new(&s.namespace),
			port: s.port.map(|p| p as u16),
		})),
		Some(tgt::Kind::ListenerSet(ls)) => Ok(PolicyTarget::ListenerSet(ListenerSetTarget {
			name: strng::new(&ls.name),
			namespace: strng::new(&ls.namespace),
			section: ls.section.as_deref().map(strng::new),
		})),
		None => Err(ProtoError::MissingRequiredField),
	}
}

pub(crate) fn targeted_policy_from_proto(
	p: &proto::agent::Policy,
	diagnostics: &mut Diagnostics,
	oidc_cookie_encoder: Option<&OidcCookieEncoder>,
) -> Result<TargetedPolicy, ProtoError> {
	use crate::types::proto::agent::policy as pol;

	let target = p
		.target
		.as_ref()
		.ok_or(ProtoError::MissingRequiredField)
		.and_then(policy_target_from_proto)?;

	let policy = match &p.kind {
		Some(pol::Kind::Traffic(spec)) => agent::PolicyType::Traffic(
			xds_phased_traffic_policy_from_proto(spec, diagnostics, oidc_cookie_encoder)?,
		),
		Some(pol::Kind::Backend(spec)) => {
			agent::PolicyType::Backend(backend_policy_from_proto(spec, diagnostics)?)
		},
		Some(pol::Kind::Frontend(spec)) => {
			agent::PolicyType::Frontend(frontend_policy_from_proto(spec, diagnostics)?)
		},
		Some(pol::Kind::Conditional(cond)) => {
			conditional_policy_from_proto(cond, diagnostics, oidc_cookie_encoder)?
		},
		None => return Err(ProtoError::MissingRequiredField),
	};

	Ok(TargetedPolicy {
		key: strng::new(&p.key),
		name: p.name.as_ref().map(Into::into),
		target,
		policy,
	})
}

fn conditional_policy_from_proto(
	cond: &proto::agent::ConditionalPolicies,
	diagnostics: &mut Diagnostics,
	oidc_cookie_encoder: Option<&OidcCookieEncoder>,
) -> Result<PolicyType, ProtoError> {
	use crate::types::proto::agent::conditional_policy as cp;

	let mut traffic = Vec::new();
	let mut expected_shape: Option<(&'static str, PolicyPhase)> = None;
	for policy in &cond.policies {
		let Some(kind) = &policy.kind else {
			return Err(ProtoError::MissingRequiredField);
		};
		match kind {
			cp::Kind::Traffic(spec) => {
				let traffic_policy =
					xds_phased_traffic_policy_from_proto(spec, diagnostics, oidc_cookie_encoder)?;
				let policy_kind = traffic_policy_kind_name(&traffic_policy.policy);
				let policy_phase = traffic_policy.phase;
				if let Some((expected_kind, expected_phase)) = expected_shape {
					if expected_kind != policy_kind {
						return Err(ProtoError::Generic(format!(
							"conditional policies must all have the same traffic policy kind; found {policy_kind}, expected {expected_kind}",
						)));
					}
					if expected_phase != policy_phase {
						return Err(ProtoError::Generic(format!(
							"conditional policies must all have the same traffic policy phase; found {policy_phase:?}, expected {expected_phase:?}",
						)));
					}
				} else {
					expected_shape = Some((policy_kind, policy_phase));
				}
				let condition = policy.condition.as_deref().map(|condition| {
					permissive_cel_expression_arc(diagnostics, "policy.conditional.condition", condition)
				});
				traffic.push((condition, traffic_policy));
			},
		}
	}

	if traffic.is_empty() {
		return Err(ProtoError::MissingRequiredField);
	}
	let Some((_, phase)) = expected_shape else {
		return Err(ProtoError::MissingRequiredField);
	};
	Ok(PolicyType::Traffic(PhasedTrafficPolicy {
		phase,
		policy: conditional_traffic_policy_to_policy(traffic)?,
	}))
}

fn conditional_traffic_policy_to_policy(
	policies: Vec<(Option<Arc<cel::Expression>>, PhasedTrafficPolicy)>,
) -> Result<TrafficPolicy, ProtoError> {
	macro_rules! build {
		($variant:ident) => {{
			let mut inners = Vec::with_capacity(policies.len());
			for (condition, policy) in policies {
				let TrafficPolicy::$variant(request_policy) = policy.policy else {
					return Err(ProtoError::Generic(
						"conditional policies must all have the same traffic policy kind".to_string(),
					));
				};
				inners.extend(
					request_policy
						.into_policy_inners()
						.into_iter()
						.map(|mut inner| {
							inner.condition = condition.clone();
							inner
						}),
				);
			}
			Ok(TrafficPolicy::$variant(RequestPolicy::from_policy_inners(
				inners,
			)))
		}};
	}

	// We can just check the type of the first one because we verified before they are all the same
	match &policies[0].1.policy {
		TrafficPolicy::ExtAuthz(_) => build!(ExtAuthz),
		TrafficPolicy::ExtProc(_) => build!(ExtProc),
		TrafficPolicy::LocalRateLimit(_) => build!(LocalRateLimit),
		TrafficPolicy::RemoteRateLimit(_) => build!(RemoteRateLimit),
		TrafficPolicy::JwtAuth(_) => build!(JwtAuth),
		TrafficPolicy::Oidc(_) => build!(Oidc),
		TrafficPolicy::BasicAuth(_) => build!(BasicAuth),
		TrafficPolicy::APIKey(_) => build!(APIKey),
		TrafficPolicy::Transformation(_) => build!(Transformation),
		TrafficPolicy::Csrf(_) => build!(Csrf),
		TrafficPolicy::RequestHeaderModifier(_) => build!(RequestHeaderModifier),
		TrafficPolicy::RequestRedirect(_) => build!(RequestRedirect),
		TrafficPolicy::UrlRewrite(_) => build!(UrlRewrite),
		TrafficPolicy::DirectResponse(_) => build!(DirectResponse),
		TrafficPolicy::CORS(_) => build!(CORS),
		other => Err(ProtoError::Generic(format!(
			"conditional traffic policy kind {} is not supported",
			traffic_policy_kind_name(other)
		))),
	}
}

fn traffic_policy_kind_name(policy: &TrafficPolicy) -> &'static str {
	match policy {
		TrafficPolicy::Timeout(_) => "timeout",
		TrafficPolicy::Retry(_) => "retry",
		TrafficPolicy::AI(_) => "ai",
		TrafficPolicy::Authorization(_) => "authorization",
		TrafficPolicy::LocalRateLimit(_) => "localRateLimit",
		TrafficPolicy::RemoteRateLimit(_) => "remoteRateLimit",
		TrafficPolicy::ExtAuthz(_) => "extAuthz",
		TrafficPolicy::ExtProc(_) => "extProc",
		TrafficPolicy::JwtAuth(_) => "jwt",
		TrafficPolicy::Oidc(_) => "oidc",
		TrafficPolicy::BasicAuth(_) => "basicAuth",
		TrafficPolicy::APIKey(_) => "apiKey",
		TrafficPolicy::Transformation(_) => "transformation",
		TrafficPolicy::Csrf(_) => "csrf",
		TrafficPolicy::RequestHeaderModifier(_) => "requestHeaderModifier",
		TrafficPolicy::ResponseHeaderModifier(_) => "responseHeaderModifier",
		TrafficPolicy::RequestRedirect(_) => "requestRedirect",
		TrafficPolicy::UrlRewrite(_) => "urlRewrite",
		TrafficPolicy::HostRewrite(_) => "hostRewrite",
		TrafficPolicy::RequestMirror(_) => "requestMirror",
		TrafficPolicy::DirectResponse(_) => "directResponse",
		TrafficPolicy::CORS(_) => "cors",
	}
}

impl From<&proto::agent::ResourceName> for ResourceName {
	fn from(value: &proto::agent::ResourceName) -> Self {
		ResourceName {
			name: strng::new(&value.name),
			namespace: strng::new(&value.namespace),
		}
	}
}

impl From<&proto::agent::TypedResourceName> for TypedResourceName {
	fn from(value: &proto::agent::TypedResourceName) -> Self {
		TypedResourceName {
			name: strng::new(&value.name),
			namespace: strng::new(&value.namespace),
			kind: strng::new(&value.kind),
		}
	}
}

impl From<&proto::agent::RouteName> for RouteName {
	fn from(value: &proto::agent::RouteName) -> Self {
		RouteName {
			name: strng::new(&value.name),
			namespace: strng::new(&value.namespace),
			rule_name: value.rule_name.as_ref().map(Into::into),
			kind: (!value.kind.is_empty()).then(|| strng::new(&value.kind)),
		}
	}
}

impl From<&proto::agent::ListenerName> for ListenerName {
	fn from(value: &proto::agent::ListenerName) -> Self {
		ListenerName {
			gateway_name: strng::new(&value.gateway_name),
			gateway_namespace: strng::new(&value.gateway_namespace),
			listener_name: strng::new(&value.listener_name),
			listener_set: value.listener_set.as_ref().map(Into::into),
		}
	}
}

fn resolve_simple_reference(
	target: Option<&proto::agent::BackendReference>,
) -> SimpleBackendReference {
	let Some(target) = target else {
		return SimpleBackendReference::Invalid;
	};
	match target.kind.as_ref() {
		None => SimpleBackendReference::Invalid,
		Some(proto::agent::backend_reference::Kind::Service(svc)) => {
			let ns = NamespacedHostname {
				namespace: strng::new(&svc.namespace),
				hostname: strng::new(&svc.hostname),
			};
			SimpleBackendReference::Service {
				name: ns,
				port: target.port as u16,
			}
		},
		Some(proto::agent::backend_reference::Kind::Backend(name)) => {
			SimpleBackendReference::Backend(name.into())
		},
	}
}

fn convert_message(
	m: &proto::agent::backend_policy_spec::ai::Message,
) -> llm::SimpleChatCompletionMessage {
	llm::SimpleChatCompletionMessage {
		role: strng::new(&m.role),
		content: strng::new(&m.content),
	}
}

fn convert_prompt_enrichment(
	prompts: &proto::agent::backend_policy_spec::ai::PromptEnrichment,
) -> llm::policy::PromptEnrichment {
	llm::policy::PromptEnrichment {
		append: prompts.append.iter().map(convert_message).collect(),
		prepend: prompts.prepend.iter().map(convert_message).collect(),
	}
}

fn convert_prompt_caching(
	pc: &proto::agent::backend_policy_spec::ai::PromptCaching,
) -> llm::policy::PromptCachingConfig {
	llm::policy::PromptCachingConfig {
		cache_system: pc.cache_system,
		cache_messages: pc.cache_messages,
		cache_tools: pc.cache_tools,
		min_tokens: pc.min_tokens.map(|t| t as usize),
		cache_message_offset: pc.cache_message_offset.unwrap_or(0) as usize,
	}
}

fn convert_webhook(
	w: &proto::agent::backend_policy_spec::ai::Webhook,
	diagnostics: &mut Diagnostics,
) -> Result<llm::policy::Webhook, ProtoError> {
	let target = resolve_simple_reference(w.backend.as_ref());

	let forward_header_matches = convert_header_match(
		diagnostics,
		"backend.ai.webhook.forwardHeaderMatches",
		&w.forward_header_matches,
	)?;

	Ok(llm::policy::Webhook {
		target,
		forward_header_matches,
	})
}

fn convert_regex_rules(
	rr: &proto::agent::backend_policy_spec::ai::RegexRules,
	diagnostics: &mut Diagnostics,
) -> llm::policy::RegexRules {
	let action_kind = proto::agent::backend_policy_spec::ai::ActionKind::try_from(rr.action).ok();
	let action = match action_kind {
		Some(ActionKind::ActionUnspecified) | Some(ActionKind::Mask) | None => {
			llm::policy::Action::Mask
		},
		Some(ActionKind::Reject) => llm::policy::Action::Reject,
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
								llm::policy::Builtin::Ssn
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::CreditCard => {
								llm::policy::Builtin::CreditCard
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::PhoneNumber => {
								llm::policy::Builtin::PhoneNumber
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::Email => {
								llm::policy::Builtin::Email
							},
							proto::agent::backend_policy_spec::ai::BuiltinRegexRule::CaSin => {
								llm::policy::Builtin::CaSin
							},
							_ => {
								diagnostics.add_warning(format!("unknown builtin regex rule value {b}; skipping"));
								return None;
							},
						};
						Some(llm::policy::RegexRule::Builtin { builtin })
					},
					Err(_) => {
						diagnostics.add_warning(format!("invalid builtin regex rule value {b}; skipping"));
						None
					},
				}
			},
			Some(proto::agent::backend_policy_spec::ai::regex_rule::Kind::Regex(n)) => {
				match regex::Regex::new(n) {
					Ok(pattern) => Some(llm::policy::RegexRule::Regex { pattern }),
					Err(err) => {
						diagnostics.add_warning(format!("invalid regex pattern {n:?}: {err}; skipping"));
						None
					},
				}
			},
			None => None,
		})
		.collect();
	llm::policy::RegexRules { action, rules }
}

fn resolve_reference(target: Option<&proto::agent::BackendReference>) -> BackendReference {
	let Some(target) = target else {
		return BackendReference::Invalid;
	};
	match target.kind.as_ref() {
		None => BackendReference::Invalid,
		Some(proto::agent::backend_reference::Kind::Service(svc)) => {
			let ns = NamespacedHostname {
				namespace: strng::new(&svc.namespace),
				hostname: strng::new(&svc.hostname),
			};
			BackendReference::Service {
				name: ns,
				port: target.port as u16,
			}
		},
		Some(proto::agent::backend_reference::Kind::Backend(name)) => {
			BackendReference::Backend(name.into())
		},
	}
}

fn convert_header_match(
	diagnostics: &mut Diagnostics,
	context: &str,
	h: &[proto::agent::HeaderMatch],
) -> Result<Vec<HeaderMatch>, ProtoError> {
	let headers = h
		.iter()
		.map(|h| match &h.value {
			None => Err(ProtoError::Generic(
				"invalid header match value".to_string(),
			)),
			Some(proto::agent::header_match::Value::Exact(e)) => Ok(HeaderMatch {
				name: crate::http::HeaderOrPseudo::try_from(h.name.as_str())?,
				value: HeaderValueMatch::Exact(crate::http::HeaderValue::from_bytes(e.as_bytes())?),
			}),
			Some(proto::agent::header_match::Value::Regex(e)) => Ok(HeaderMatch {
				name: crate::http::HeaderOrPseudo::try_from(h.name.as_str())?,
				value: regex_or_warn_invalid(diagnostics, format!("{context}.{}", h.name), e)
					.map(HeaderValueMatch::Regex)
					.unwrap_or(HeaderValueMatch::Invalid),
			}),
		})
		.collect::<Result<Vec<_>, _>>()?;
	Ok(headers)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;
	use crate::store::RequestPolicyTrait;
	use crate::types::proto::agent::backend_policy_spec::Ai;

	fn test_policy_target() -> proto::agent::PolicyTarget {
		proto::agent::PolicyTarget {
			kind: Some(proto::agent::policy_target::Kind::Route(
				proto::agent::policy_target::RouteTarget {
					name: "route".to_string(),
					namespace: "default".to_string(),
					route_rule: None,
					kind: "HTTPRoute".to_string(),
				},
			)),
		}
	}

	fn conditional_traffic_policy(
		condition: &str,
		kind: proto::agent::traffic_policy_spec::Kind,
	) -> proto::agent::ConditionalPolicy {
		proto::agent::ConditionalPolicy {
			condition: Some(condition.to_string()),
			kind: Some(proto::agent::conditional_policy::Kind::Traffic(
				proto::agent::TrafficPolicySpec {
					phase: proto::agent::traffic_policy_spec::PolicyPhase::Route as i32,
					kind: Some(kind),
				},
			)),
		}
	}

	fn fallback_conditional_traffic_policy(
		kind: proto::agent::traffic_policy_spec::Kind,
	) -> proto::agent::ConditionalPolicy {
		proto::agent::ConditionalPolicy {
			condition: None,
			kind: Some(proto::agent::conditional_policy::Kind::Traffic(
				proto::agent::TrafficPolicySpec {
					phase: proto::agent::traffic_policy_spec::PolicyPhase::Route as i32,
					kind: Some(kind),
				},
			)),
		}
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_traffic_same_kind() -> Result<(), ProtoError> {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![
						conditional_traffic_policy(
							"request.path == '/a'",
							proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
								proto::agent::HeaderModifier::default(),
							),
						),
						conditional_traffic_policy(
							"request.path == '/b'",
							proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
								proto::agent::HeaderModifier::default(),
							),
						),
					],
				},
			)),
		};

		let policy = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), None)?;
		let PolicyType::Traffic(PhasedTrafficPolicy {
			policy: TrafficPolicy::RequestHeaderModifier(policies),
			..
		}) = policy.policy
		else {
			panic!("expected conditional request header modifier policy");
		};
		assert_eq!(policies.iter().count(), 2);
		Ok(())
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_empty_condition_is_fallback()
	-> Result<(), ProtoError> {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![
						conditional_traffic_policy(
							"request.path == '/a'",
							proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
								proto::agent::HeaderModifier::default(),
							),
						),
						fallback_conditional_traffic_policy(
							proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
								proto::agent::HeaderModifier::default(),
							),
						),
					],
				},
			)),
		};

		let policy = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), None)?;
		let PolicyType::Traffic(PhasedTrafficPolicy {
			policy: TrafficPolicy::RequestHeaderModifier(policies),
			..
		}) = policy.policy
		else {
			panic!("expected conditional request header modifier policy");
		};
		let entries = policies.iter().collect::<Vec<_>>();
		assert_eq!(entries.len(), 2);
		assert!(entries[0].condition.is_some());
		assert!(entries[1].condition.is_none());
		Ok(())
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_invalid_condition_never_matches()
	-> Result<(), ProtoError> {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![conditional_traffic_policy(
						"request.path ==",
						proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
							proto::agent::HeaderModifier::default(),
						),
					)],
				},
			)),
		};

		let mut diagnostics = Diagnostics::default();
		let policy = targeted_policy_from_proto(&policy, &mut diagnostics, None)?;
		let PolicyType::Traffic(PhasedTrafficPolicy {
			policy: TrafficPolicy::RequestHeaderModifier(policies),
			..
		}) = policy.policy
		else {
			panic!("expected conditional request header modifier policy");
		};
		let entries = policies.iter().collect::<Vec<_>>();
		assert_eq!(entries.len(), 1);
		let condition = entries[0]
			.condition
			.as_ref()
			.expect("non-empty invalid condition should remain conditional");
		assert_eq!(condition.original_expression, "request.path ==");
		assert!(!crate::cel::Executor::new_empty().eval_bool(condition));
		assert_eq!(diagnostics.into_warnings().len(), 1);
		Ok(())
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_rate_limit() -> Result<(), ProtoError> {
		let local_rate_limit = || {
			proto::agent::traffic_policy_spec::Kind::LocalRateLimit(
				proto::agent::traffic_policy_spec::LocalRateLimit {
					max_tokens: 10,
					tokens_per_fill: 10,
					fill_interval: Some(prost_types::Duration {
						seconds: 1,
						nanos: 0,
					}),
					r#type: proto::agent::traffic_policy_spec::local_rate_limit::Type::Token as i32,
				},
			)
		};
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![
						conditional_traffic_policy("request.path == '/a'", local_rate_limit()),
						conditional_traffic_policy("request.path == '/b'", local_rate_limit()),
					],
				},
			)),
		};

		let policy = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), None)?;
		let PolicyType::Traffic(PhasedTrafficPolicy {
			policy: TrafficPolicy::LocalRateLimit(policies),
			..
		}) = policy.policy
		else {
			panic!("expected conditional local rate limit policy");
		};
		assert_eq!(policies.iter().count(), 2);
		Ok(())
	}

	#[test]
	fn test_targeted_policy_from_proto_rejects_mixed_conditional_traffic_kinds() {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![
						conditional_traffic_policy(
							"request.path == '/a'",
							proto::agent::traffic_policy_spec::Kind::RequestHeaderModifier(
								proto::agent::HeaderModifier::default(),
							),
						),
						conditional_traffic_policy(
							"request.path == '/b'",
							proto::agent::traffic_policy_spec::Kind::RequestRedirect(
								proto::agent::RequestRedirect::default(),
							),
						),
					],
				},
			)),
		};

		let err = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), None)
			.expect_err("mixed conditional traffic kinds should be rejected");
		assert!(
			err
				.to_string()
				.contains("must all have the same traffic policy kind")
		);
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_oidc() -> Result<(), ProtoError> {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![
						conditional_traffic_policy(
							"request.path.startsWith('/admin')",
							proto::agent::traffic_policy_spec::Kind::Oidc(sample_oidc_proto()),
						),
						conditional_traffic_policy(
							"request.path.startsWith('/internal')",
							proto::agent::traffic_policy_spec::Kind::Oidc(sample_oidc_proto()),
						),
					],
				},
			)),
		};

		let encoder = test_oidc_cookie_encoder();
		let policy = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), Some(&encoder))?;
		let PolicyType::Traffic(PhasedTrafficPolicy {
			policy: TrafficPolicy::Oidc(policies),
			..
		}) = policy.policy
		else {
			panic!("expected conditional OIDC policy");
		};
		let entries = policies.iter().collect::<Vec<_>>();
		assert_eq!(entries.len(), 2);
		assert!(entries[0].condition.is_some());
		assert!(entries[1].condition.is_some());
		Ok(())
	}

	#[test]
	fn test_targeted_policy_from_proto_conditional_oidc_requires_session_key() {
		let policy = proto::agent::Policy {
			key: "policy".to_string(),
			name: None,
			target: Some(test_policy_target()),
			kind: Some(proto::agent::policy::Kind::Conditional(
				proto::agent::ConditionalPolicies {
					policies: vec![conditional_traffic_policy(
						"request.path.startsWith('/admin')",
						proto::agent::traffic_policy_spec::Kind::Oidc(sample_oidc_proto()),
					)],
				},
			)),
		};

		let err = targeted_policy_from_proto(&policy, &mut Diagnostics::default(), None)
			.expect_err("conditional OIDC without encoder must be rejected");
		assert!(
			err.to_string().contains("SESSION_KEY"),
			"expected SESSION_KEY rejection, got: {err}",
		);
	}

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

		let policy = traffic_policy_from_proto(&spec, &mut Diagnostics::default())?;

		if let TrafficPolicy::Csrf(_csrf_policy) = policy {
			// We can't directly access the HashSet since it's private, but we can test
			// the policy works by creating a test that would use the contains() method
			// This verifies the conversion worked and the HashSet deduplication happened

			// For now, just verify we got a CSRF policy
			// In a real implementation, you'd add a test helper method to the Csrf struct
			// to verify the contents
			Ok(())
		} else {
			panic!("Expected CSRF policy variant, got: {policy:?}");
		}
	}

	#[test]
	fn test_backend_policy_spec_to_ai_policy() -> Result<(), ProtoError> {
		use proto::agent::backend_policy_spec::ai::RouteType;

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
				transformations: vec![(
					"system".to_string(),
					"\"Always answer in JSON\"".to_string(),
				)]
				.into_iter()
				.collect(),
				prompt_guard: None,
				prompts: None,
				model_aliases: Default::default(),
				prompt_caching: None,
				routes: vec![
					(
						"/v1/chat/completions".to_string(),
						RouteType::Completions as i32,
					),
					("/v1/messages".to_string(), RouteType::Messages as i32),
					("/v1/detect".to_string(), RouteType::Detect as i32),
				]
				.into_iter()
				.collect(),
			})),
		};

		let policy = backend_policy_from_proto(&spec, &mut Diagnostics::default())?;

		if let BackendTrafficPolicy::AI(ai_policy) = policy {
			let defaults = ai_policy.defaults.as_ref().expect("defaults should be set");
			let overrides = ai_policy
				.overrides
				.as_ref()
				.expect("overrides should be set");
			let transformation_policy = ai_policy
				.transformations
				.as_ref()
				.expect("transformation_policy should be set");

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
			assert!(transformation_policy.get("system").is_some());

			// Verify routes conversion
			assert_eq!(ai_policy.routes.len(), 3);
			assert_eq!(
				ai_policy.routes.get("/v1/chat/completions"),
				Some(&llm::RouteType::Completions)
			);
			assert_eq!(
				ai_policy.routes.get("/v1/messages"),
				Some(&llm::RouteType::Messages)
			);
			assert_eq!(
				ai_policy.routes.get("/v1/detect"),
				Some(&llm::RouteType::Detect)
			);
		} else {
			panic!("Expected AI policy variant");
		}

		Ok(())
	}

	#[test]
	fn test_backend_policy_spec_to_transformation_policy() -> Result<(), ProtoError> {
		let spec = proto::agent::BackendPolicySpec {
			kind: Some(proto::agent::backend_policy_spec::Kind::Transformation(
				proto::agent::traffic_policy_spec::TransformationPolicy {
					request: Some(
						proto::agent::traffic_policy_spec::transformation_policy::Transform {
							set: vec![proto::agent::traffic_policy_spec::HeaderTransformation {
								name: "x-backend-req".to_string(),
								expression: "\"backend-req\"".to_string(),
							}],
							..Default::default()
						},
					),
					response: Some(
						proto::agent::traffic_policy_spec::transformation_policy::Transform {
							add: vec![proto::agent::traffic_policy_spec::HeaderTransformation {
								name: "x-backend-resp".to_string(),
								expression: "\"backend-resp\"".to_string(),
							}],
							..Default::default()
						},
					),
				},
			)),
		};

		let policy = backend_policy_from_proto(&spec, &mut Diagnostics::default())?;
		let BackendTrafficPolicy::Transformation(transformation) = policy else {
			panic!("Expected Transformation policy variant");
		};
		assert_eq!(transformation.expressions().count(), 2);
		Ok(())
	}

	#[test]
	fn test_backend_kind_aws_conversion() -> Result<(), ProtoError> {
		use proto::agent::aws_backend::Service;

		let arn = "arn:aws:bedrock-agentcore:us-east-1:123456789012:runtime/abc123".to_string();
		let qualifier = Some("v1".to_string());
		let proto_backend = proto::agent::Backend {
			key: "test-ns/aws-backend".to_string(),
			name: Some(proto::agent::ResourceName {
				name: "aws-backend".to_string(),
				namespace: "test-ns".to_string(),
			}),
			kind: Some(proto::agent::backend::Kind::Aws(proto::agent::AwsBackend {
				service: Some(Service::AgentCore(proto::agent::AwsAgentCoreBackend {
					agent_runtime_arn: arn.clone(),
					qualifier: qualifier.clone(),
				})),
			})),
			inline_policies: vec![],
		};

		let bw = backend_with_policies_from_proto(&proto_backend, &mut Diagnostics::default())?;
		let Backend::Aws(name, config) = &bw.backend else {
			panic!("Expected Backend::Aws, got {:?}", bw.backend);
		};
		assert_eq!(name.to_string(), "test-ns/aws-backend");
		assert_eq!(config.region(), "us-east-1");
		assert_eq!(config.service_name(), "bedrock-agentcore");
		assert_eq!(
			config.get_host(),
			"bedrock-agentcore.us-east-1.amazonaws.com"
		);
		let path = config.get_path();
		assert!(path.starts_with("/runtimes/"));
		assert!(path.contains("qualifier=v1"));
		Ok(())
	}

	#[tokio::test]
	async fn test_vertex_provider_empty_region_is_none() -> Result<(), ProtoError> {
		use proto::agent::ai_backend::Vertex;
		use proto::agent::ai_backend::provider::Provider;

		let proto_backend = proto::agent::Backend {
			key: "test-ns/vertex-backend".to_string(),
			name: Some(proto::agent::ResourceName {
				name: "vertex-backend".to_string(),
				namespace: "test-ns".to_string(),
			}),
			kind: Some(proto::agent::backend::Kind::Ai(proto::agent::AiBackend {
				provider_groups: vec![proto::agent::ai_backend::ProviderGroup {
					providers: vec![proto::agent::ai_backend::Provider {
						name: "vertex".to_string(),
						host_override: None,
						path_override: None,
						path_prefix: None,
						provider: Some(Provider::Vertex(Vertex {
							model: None,
							region: "".to_string(),
							project_id: "my-project".to_string(),
						})),
						inline_policies: vec![],
					}],
				}],
			})),
			inline_policies: vec![],
		};

		let bw = backend_with_policies_from_proto(&proto_backend, &mut Diagnostics::default())?;
		let Backend::AI(_, ai_backend) = &bw.backend else {
			panic!("Expected Backend::AI, got {:?}", bw.backend);
		};
		let providers = ai_backend.providers.iter();
		let (provider, _) = providers.iter().next().unwrap();
		let AIProvider::Vertex(vertex) = &provider.provider else {
			panic!("Expected AIProvider::Vertex");
		};
		assert!(vertex.region.is_none(), "empty region should map to None");
		Ok(())
	}

	#[tokio::test]
	async fn test_vertex_provider_with_region() -> Result<(), ProtoError> {
		use proto::agent::ai_backend::Vertex;
		use proto::agent::ai_backend::provider::Provider;

		let proto_backend = proto::agent::Backend {
			key: "test-ns/vertex-backend".to_string(),
			name: Some(proto::agent::ResourceName {
				name: "vertex-backend".to_string(),
				namespace: "test-ns".to_string(),
			}),
			kind: Some(proto::agent::backend::Kind::Ai(proto::agent::AiBackend {
				provider_groups: vec![proto::agent::ai_backend::ProviderGroup {
					providers: vec![proto::agent::ai_backend::Provider {
						name: "vertex".to_string(),
						host_override: None,
						path_override: None,
						path_prefix: None,
						provider: Some(Provider::Vertex(Vertex {
							model: None,
							region: "us-central1".to_string(),
							project_id: "my-project".to_string(),
						})),
						inline_policies: vec![],
					}],
				}],
			})),
			inline_policies: vec![],
		};

		let bw = backend_with_policies_from_proto(&proto_backend, &mut Diagnostics::default())?;
		let Backend::AI(_, ai_backend) = &bw.backend else {
			panic!("Expected Backend::AI, got {:?}", bw.backend);
		};
		let providers = ai_backend.providers.iter();
		let (provider, _) = providers.iter().next().unwrap();
		let AIProvider::Vertex(vertex) = &provider.provider else {
			panic!("Expected AIProvider::Vertex");
		};
		assert_eq!(vertex.region.as_deref(), Some("us-central1"));
		Ok(())
	}

	#[test]
	fn test_frontend_policy_spec_metrics() -> Result<(), ProtoError> {
		use crate::types::proto::agent::frontend_policy_spec as fps;

		let spec = proto::agent::FrontendPolicySpec {
			kind: Some(fps::Kind::Metrics(fps::Metrics {
				fields: Some(fps::metrics::Fields {
					add: vec![
						fps::metrics::Field {
							name: "team".to_string(),
							expression: "jwt.team".to_string(),
						},
						fps::metrics::Field {
							name: "org".to_string(),
							expression: r#"request.headers["x-org-id"]"#.to_string(),
						},
					],
				}),
			})),
		};

		let mut diag = Diagnostics::default();
		let policy = frontend_policy_from_proto(&spec, &mut diag)?;
		let FrontendPolicy::Metrics(metrics) = policy else {
			panic!("Expected Metrics policy variant, got: {policy:?}");
		};

		assert_eq!(metrics.add.len(), 2);
		assert!(metrics.add.contains_key("team"), "expected team field");
		assert!(metrics.add.contains_key("org"), "expected org field");
		Ok(())
	}

	#[test]
	fn test_frontend_policy_spec_metrics_empty_fields() -> Result<(), ProtoError> {
		use crate::types::proto::agent::frontend_policy_spec as fps;

		let spec = proto::agent::FrontendPolicySpec {
			kind: Some(fps::Kind::Metrics(fps::Metrics { fields: None })),
		};

		let mut diag = Diagnostics::default();
		let policy = frontend_policy_from_proto(&spec, &mut diag)?;
		let FrontendPolicy::Metrics(metrics) = policy else {
			panic!("Expected Metrics policy variant, got: {policy:?}");
		};

		assert_eq!(metrics.add.len(), 0);
		Ok(())
	}

	fn sample_oidc_proto() -> proto::agent::traffic_policy_spec::Oidc {
		use proto::agent::traffic_policy_spec::oidc::TokenEndpointAuth;
		proto::agent::traffic_policy_spec::Oidc {
			policy_id: "policy/default/my-policy:oidc".to_string(),
			issuer: "https://issuer.example.com".to_string(),
			authorization_endpoint: "https://issuer.example.com/authorize".to_string(),
			token_endpoint: "https://issuer.example.com/token".to_string(),
			token_endpoint_auth: TokenEndpointAuth::ClientSecretBasic as i32,
			jwks_inline: serde_json::json!({
				"keys": [{
					"use": "sig",
					"kty": "EC",
					"kid": "kid-1",
					"crv": "P-256",
					"alg": "ES256",
					"x": "WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk",
					"y": "xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"
				}]
			})
			.to_string(),
			client_id: "client-id".to_string(),
			client_secret: "client-secret".to_string(),
			redirect_uri: "https://app.example.com/oauth/callback".to_string(),
			scopes: vec!["openid".to_string(), "profile".to_string()],
		}
	}

	fn test_oidc_cookie_encoder() -> OidcCookieEncoder {
		let session_encoder = crate::http::sessionpersistence::Encoder::aes(
			"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		)
		.expect("aes encoder");
		OidcCookieEncoder::from_session_encoder(&session_encoder).expect("oidc cookie encoder")
	}

	fn oidc_traffic_spec(
		oidc: proto::agent::traffic_policy_spec::Oidc,
	) -> proto::agent::TrafficPolicySpec {
		proto::agent::TrafficPolicySpec {
			phase: proto::agent::traffic_policy_spec::PolicyPhase::Route as i32,
			kind: Some(proto::agent::traffic_policy_spec::Kind::Oidc(oidc)),
		}
	}

	fn assert_oidc_proto_rejected(oidc: proto::agent::traffic_policy_spec::Oidc, expected: &str) {
		let spec = oidc_traffic_spec(oidc);
		let encoder = test_oidc_cookie_encoder();
		let err =
			match xds_traffic_policy_from_proto(&spec, &mut Diagnostics::default(), Some(&encoder)) {
				Ok(_) => panic!("expected xDS OIDC policy to be rejected"),
				Err(err) => err,
			};
		let msg = err.to_string();
		assert!(
			msg.contains(expected),
			"expected error containing {expected:?}, got: {msg}",
		);
	}

	#[test]
	fn test_oidc_spec_compiles_to_traffic_policy() -> Result<(), ProtoError> {
		let spec = oidc_traffic_spec(sample_oidc_proto());

		let encoder = test_oidc_cookie_encoder();
		let policy = xds_traffic_policy_from_proto(&spec, &mut Diagnostics::default(), Some(&encoder))?;
		let TrafficPolicy::Oidc(_) = &policy else {
			panic!("Expected Oidc");
		};
		Ok(())
	}

	#[test]
	fn test_oidc_spec_requires_session_key() {
		let spec = oidc_traffic_spec(sample_oidc_proto());

		let err = match xds_traffic_policy_from_proto(&spec, &mut Diagnostics::default(), None) {
			Ok(_) => panic!("expected missing SESSION_KEY"),
			Err(err) => err,
		};
		assert!(
			err.to_string().contains("SESSION_KEY"),
			"unexpected error: {err}",
		);
	}

	#[test]
	fn test_oidc_spec_rejects_invalid_policy_id() {
		let mut oidc = sample_oidc_proto();
		oidc.policy_id = "not-a-valid-prefix".into();
		assert_oidc_proto_rejected(oidc, "invalid policy_id");
	}

	#[test]
	fn test_oidc_spec_rejects_empty_policy_id() {
		let mut oidc = sample_oidc_proto();
		oidc.policy_id = String::new();
		assert_oidc_proto_rejected(oidc, "invalid policy_id");
	}

	#[test]
	fn test_oidc_spec_rejects_unspecified_token_endpoint_auth() {
		use proto::agent::traffic_policy_spec::oidc::TokenEndpointAuth;

		let mut oidc = sample_oidc_proto();
		oidc.token_endpoint_auth = TokenEndpointAuth::Unspecified as i32;
		assert_oidc_proto_rejected(oidc, "token_endpoint_auth");
	}

	#[test]
	fn test_oidc_spec_rejects_empty_issuer() {
		let mut oidc = sample_oidc_proto();
		oidc.issuer = String::new();
		assert_oidc_proto_rejected(oidc, "issuer");
	}

	#[test]
	fn test_oidc_spec_rejects_invalid_authorization_endpoint() {
		let mut oidc = sample_oidc_proto();
		oidc.authorization_endpoint = "not-a-url".into();
		assert_oidc_proto_rejected(oidc, "authorization_endpoint");
	}

	#[test]
	fn test_oidc_spec_rejects_invalid_token_endpoint() {
		let mut oidc = sample_oidc_proto();
		oidc.token_endpoint = "file:///tmp/token".into();
		assert_oidc_proto_rejected(oidc, "token_endpoint");
	}

	#[test]
	fn test_oidc_spec_rejects_invalid_redirect_uri() {
		let mut oidc = sample_oidc_proto();
		oidc.redirect_uri = "not-a-url".into();
		assert_oidc_proto_rejected(oidc, "redirect_uri");
	}

	#[test]
	fn test_oidc_spec_rejects_empty_jwks_inline() {
		let mut oidc = sample_oidc_proto();
		oidc.jwks_inline = String::new();
		assert_oidc_proto_rejected(oidc, "jwks_inline");
	}

	#[test]
	fn test_oidc_spec_rejects_invalid_jwks_inline() {
		let mut oidc = sample_oidc_proto();
		oidc.jwks_inline = "not json".into();
		assert_oidc_proto_rejected(oidc, "inline oidc jwks");
	}

	#[test]
	fn test_oidc_spec_rejects_none_auth_with_client_secret() {
		use proto::agent::traffic_policy_spec::oidc::TokenEndpointAuth;

		let mut oidc = sample_oidc_proto();
		oidc.token_endpoint_auth = TokenEndpointAuth::None as i32;
		assert_oidc_proto_rejected(oidc, "must not carry a client_secret");
	}

	#[test]
	fn test_oidc_spec_rejects_confidential_auth_without_client_secret() {
		let mut oidc = sample_oidc_proto();
		oidc.client_secret = String::new();
		assert_oidc_proto_rejected(oidc, "requires a non-empty client_secret");
	}
}
