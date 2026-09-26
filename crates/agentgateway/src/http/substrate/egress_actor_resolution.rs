use tonic::Code;

use super::{ActorRef, TRACE_POLICY_KIND, valid_resource_name};
use crate::http::Request;
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, ProxyResponse};
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::transport::stream::{Extension, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::*;

#[derive(Clone, Debug)]
pub(crate) struct ActorIdentity {
	pub(crate) atespace: String,
	pub(crate) actor_name: String,
}

/// Validates an actor's identity before accepting a CONNECT tunnel.
#[apply(schema!)]
pub struct EgressActorResolution {
	/// Backend that receives GetActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
}

impl EgressActorResolution {
	pub(crate) fn identity(req: &Request) -> Result<ActorIdentity, ProxyError> {
		let certificate = req
			.extensions()
			.get::<TLSConnectionInfo>()
			.and_then(|tls| tls.src_identity.as_ref())
			.and_then(|identity| identity.certificate.as_deref())
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied("missing authenticated actor certificate".to_owned())
			})?;
		let pem = pem::parse(certificate.as_bytes()).map_err(|error| {
			ProxyError::SubstrateEgressDenied(format!("invalid actor certificate: {error}"))
		})?;
		let (_, certificate) =
			x509_parser::parse_x509_certificate(pem.contents()).map_err(|error| {
				ProxyError::SubstrateEgressDenied(format!("invalid actor certificate: {error}"))
			})?;
		let san = certificate.subject_alternative_name().map_err(|error| {
			ProxyError::SubstrateEgressDenied(format!("invalid actor certificate SAN: {error}"))
		})?;
		let mut uris = san
			.iter()
			.flat_map(|san| &san.value.general_names)
			.filter_map(|name| match name {
				x509_parser::extensions::GeneralName::URI(uri) => Some(*uri),
				_ => None,
			});
		let uri = uris.next().ok_or_else(|| {
			ProxyError::SubstrateEgressDenied("actor certificate has no URI SAN".to_owned())
		})?;
		if uris.next().is_some() {
			return Err(ProxyError::SubstrateEgressDenied(
				"actor certificate has multiple URI SANs".to_owned(),
			));
		}
		let (atespace, actor_name) = uri
			.strip_prefix("spiffe://substrate-actor.local/ateom-for-actor/")
			.and_then(|path| path.split_once('/'))
			.filter(|(atespace, actor)| valid_resource_name(atespace) && valid_resource_name(actor))
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied("invalid ateom-for-actor SPIFFE ID".to_owned())
			})?;
		Ok(ActorIdentity {
			atespace: atespace.to_owned(),
			actor_name: actor_name.to_owned(),
		})
	}

	pub(crate) async fn authorize_connect(
		&self,
		inputs: &Arc<ProxyInputs>,
		connection: &Extension,
		req: &mut Request,
	) -> Result<ActorIdentity, ProxyResponse> {
		connection
			.copy::<TCPConnectionInfo>(req.extensions_mut())
			.expect("tcp connection must be set");
		connection.copy::<TLSConnectionInfo>(req.extensions_mut());
		let identity = Self::identity(req)?;
		self
			.authorize(
				&PolicyClient::new(inputs.clone()).with_parent(req),
				&identity,
			)
			.await?;

		Ok(identity)
	}

	async fn authorize(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
	) -> Result<(), ProxyResponse> {
		let actor = ActorRef {
			atespace: identity.atespace.clone(),
			name: identity.actor_name.clone(),
		};
		let channel = self
			.target
			.grpc_channel(client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate));
		let mut control = protos::ateapi::control_client::ControlClient::new(channel);
		let result = crate::proxy::dtrace::scope_future(
			Some(TRACE_POLICY_KIND),
			control.get_actor(protos::ateapi::GetActorRequest {
				actor: Some(protos::ateapi::ObjectRef {
					atespace: actor.atespace.clone(),
					name: actor.name.clone(),
				}),
			}),
		)
		.await;
		let current = match result {
			Ok(response) => response.into_inner(),
			Err(status) if matches!(status.code(), Code::Unavailable | Code::DeadlineExceeded) => {
				return Err(
					ProxyError::SubstrateEgressUnavailable(format!(
						"actor identity check unavailable: {status}"
					))
					.into(),
				);
			},
			Err(status) => {
				return Err(
					ProxyError::SubstrateEgressDenied(format!("actor identity check denied: {status}"))
						.into(),
				);
			},
		};
		if current.status.as_ref().map(|status| status.state)
			!= Some(protos::ateapi::ActorState::Running as i32)
		{
			return Err(ProxyError::SubstrateEgressDenied("actor is not running".to_owned()).into());
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use rcgen::{CertificateParams, KeyPair, SanType};

	use super::*;
	use crate::http::Body;
	use crate::transport::tls::TlsInfo;

	fn request_with_identity(uris: &[&str]) -> Request {
		let mut params = CertificateParams::default();
		params.subject_alt_names = uris
			.iter()
			.map(|uri| SanType::URI((*uri).try_into().unwrap()))
			.collect();
		let certificate = params
			.self_signed(&KeyPair::generate().unwrap())
			.unwrap()
			.pem();
		let mut req = Request::new(Body::empty());
		req.extensions_mut().insert(TLSConnectionInfo {
			src_identity: Some(TlsInfo {
				certificate: Some(certificate.into()),
				..Default::default()
			}),
			..Default::default()
		});
		req
	}

	#[test]
	fn actor_identity_is_parsed_from_the_certificate() {
		let identity = EgressActorResolution::identity(&request_with_identity(&[
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor",
		]))
		.unwrap();
		assert_eq!(identity.atespace, "demo");
		assert_eq!(identity.actor_name, "my-actor");
	}

	#[test]
	fn actor_identity_requires_a_single_valid_ateom_uri() {
		let valid = "spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor";
		for uris in [
			vec![],
			vec![valid, valid],
			vec![valid, "https://example.com"],
			vec!["spiffe://substrate-actor.local/actor/demo/my-actor"],
			vec!["spiffe://substrate-actor.local/atespace/demo/actor/my-actor"],
			vec!["spiffe://other.local/ateom-for-actor/demo/my-actor"],
			vec!["https://substrate-actor.local/ateom-for-actor/demo/my-actor"],
			vec!["spiffe://user@substrate-actor.local/ateom-for-actor/demo/my-actor"],
			vec!["spiffe://substrate-actor.local:443/ateom-for-actor/demo/my-actor"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor//my-actor"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/demo/"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor/extra"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor?query"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor#fragment"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/de%2Fmo/my-actor"],
			vec!["spiffe://substrate-actor.local/ateom-for-actor/demo/UPPER"],
		] {
			assert!(
				EgressActorResolution::identity(&request_with_identity(&uris)).is_err(),
				"{uris:?}"
			);
		}
	}
}
