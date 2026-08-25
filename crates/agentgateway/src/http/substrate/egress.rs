use super::{ActorRef, valid_resource_name};
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::transport::stream::TLSConnectionInfo;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::*;

const ACTOR_SPIFFE_PREFIX: &str = "spiffe://substrate-actor.local/atespace/";

/// Authorizes an actor's egress to the hostname recovered from an internal CONNECT listener.
#[apply(schema!)]
pub struct SubstrateEgress {
	/// Backend that receives GetActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
}

impl SubstrateEgress {
	fn actor(req: &Request) -> Result<ActorRef, ProxyError> {
		let spiffe_id = req
			.extensions()
			.get::<TLSConnectionInfo>()
			.and_then(|tls| tls.src_identity.as_ref())
			.and_then(|identity| identity.spiffe_id.as_deref())
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied("missing authenticated actor SPIFFE identity".to_owned())
			})?;
		let Some((atespace, name)) = spiffe_id
			.strip_prefix(ACTOR_SPIFFE_PREFIX)
			.and_then(|path| path.split_once("/actor/"))
			.filter(|(atespace, name)| valid_resource_name(atespace) && valid_resource_name(name))
		else {
			return Err(ProxyError::SubstrateEgressDenied(
				"invalid authenticated actor SPIFFE identity".to_owned(),
			));
		};
		Ok(ActorRef {
			atespace: atespace.to_owned(),
			name: name.to_owned(),
		})
	}
}

impl RequestPolicyTrait for SubstrateEgress {
	async fn apply(
		&self,
		_client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, crate::proxy::ProxyResponse> {
		let actor = Self::actor(req)?;
		log.ate_actor_id = Some(actor.name.clone());
		log.ate_atespace = Some(actor.atespace.clone());
		// TODO: implement egress policy in Substrate. Then, we can look it up, cache it, and enforce it.
		// For now, we just add ate metadata
		Ok(PolicyResponse::default())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::Body;
	use crate::transport::tls::TlsInfo;

	#[test]
	fn actor_is_derived_from_the_authenticated_substrate_svid() {
		let mut req = Request::new(Body::empty());
		req.extensions_mut().insert(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				spiffe_id: Some(strng::literal!(
					"spiffe://substrate-actor.local/atespace/demo/actor/my-actor"
				)),
				..Default::default()
			}),
			..Default::default()
		});

		assert_eq!(
			SubstrateEgress::actor(&req).unwrap(),
			ActorRef {
				atespace: "demo".into(),
				name: "my-actor".into(),
			}
		);
	}

	#[test]
	fn actor_rejects_non_substrate_svids() {
		let mut req = Request::new(Body::empty());
		req.extensions_mut().insert(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				spiffe_id: Some(strng::literal!("spiffe://cluster.local/ns/demo/sa/default")),
				..Default::default()
			}),
			..Default::default()
		});

		assert!(SubstrateEgress::actor(&req).is_err());
	}
}
