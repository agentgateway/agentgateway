use super::{ActorRef, valid_resource_name};
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::*;

const ATESPACE_HEADER: &str = "x-ate-atespace";
const ACTOR_HEADER: &str = "x-ate-actor";
const ACTOR_VERSION_HEADER: &str = "x-ate-actor-version";

/// Authorizes an actor's egress to the hostname recovered from an internal CONNECT listener.
#[apply(schema!)]
pub struct SubstrateEgress {
	/// Backend that receives GetActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
}

impl SubstrateEgress {
	fn metadata(req: &Request) -> Result<(ActorRef, i64), ProxyError> {
		let headers = &req
			.extensions()
			.get::<crate::cel::SourceContext>()
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied(
					"request did not originate from a CONNECT tunnel".to_owned(),
				)
			})?
			.connect_headers;
		let required = |name: &'static str| -> Result<&str, ProxyError> {
			let mut values = headers.get_all(name).iter();
			let value = values.next().ok_or_else(|| {
				ProxyError::SubstrateEgressDenied(format!("missing {name} CONNECT header"))
			})?;
			if values.next().is_some() {
				return Err(ProxyError::SubstrateEgressDenied(format!(
					"multiple {name} CONNECT headers"
				)));
			}
			value
				.to_str()
				.map_err(|_| ProxyError::SubstrateEgressDenied(format!("invalid {name} CONNECT header")))
		};
		let atespace = required(ATESPACE_HEADER)?;
		let name = required(ACTOR_HEADER)?;
		if !valid_resource_name(atespace) || !valid_resource_name(name) {
			return Err(ProxyError::SubstrateEgressDenied(
				"invalid actor identity CONNECT headers".to_owned(),
			));
		}
		let version = required(ACTOR_VERSION_HEADER)?
			.parse::<i64>()
			.map_err(|_| {
				ProxyError::SubstrateEgressDenied("X-Ate-Actor-Version must be a positive int64".to_owned())
			})?;
		if version <= 0 {
			return Err(ProxyError::SubstrateEgressDenied(
				"X-Ate-Actor-Version must be a positive int64".to_owned(),
			));
		}
		Ok((
			ActorRef {
				atespace: atespace.to_owned(),
				name: name.to_owned(),
			},
			version,
		))
	}
}

impl RequestPolicyTrait for SubstrateEgress {
	async fn apply(
		&self,
		_client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, crate::proxy::ProxyResponse> {
		let (actor, _minimum_version) = Self::metadata(req)?;
		log.ate_actor_id = Some(actor.name.clone());
		log.ate_atespace = Some(actor.atespace.clone());
		// TODO: implement egress policy in Substrate. Then, we can look it up, cache it, and enforce it.
		// For now, we just add ate metadata
		Ok(PolicyResponse::default())
	}
}
