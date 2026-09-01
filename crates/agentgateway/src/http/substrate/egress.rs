use serde::Deserialize;
use tonic::Code;

use super::{ActorRef, TRACE_POLICY_KIND, valid_resource_name};
use crate::http::Request;
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, ProxyResponse};
use crate::telemetry::log;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::transport::stream::{Extension, TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::*;

const ACTOR_IDENTITY_OID: &str = "1.3.6.1.4.1.11129.2.12.2";

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ActorIdentity {
	atespace: String,
	actor_name: String,
	actor_uid: String,
	purpose: String,
}

/// Validates an actor's identity before accepting a CONNECT tunnel.
#[apply(schema!)]
pub struct SubstrateEgress {
	/// Backend that receives GetActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
}

impl SubstrateEgress {
	fn identity(req: &Request) -> Result<ActorIdentity, ProxyError> {
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
		let mut extensions = certificate
			.extensions()
			.iter()
			.filter(|extension| extension.oid.to_id_string() == ACTOR_IDENTITY_OID);
		let extension = extensions.next().ok_or_else(|| {
			ProxyError::SubstrateEgressDenied("actor certificate has no ActorIdentity".to_owned())
		})?;
		if extensions.next().is_some() {
			return Err(ProxyError::SubstrateEgressDenied(
				"actor certificate has multiple ActorIdentity extensions".to_owned(),
			));
		}
		let identity: ActorIdentity = serde_json::from_slice(extension.value).map_err(|error| {
			ProxyError::SubstrateEgressDenied(format!("invalid ActorIdentity: {error}"))
		})?;
		if !valid_resource_name(&identity.atespace)
			|| !valid_resource_name(&identity.actor_name)
			|| identity.actor_uid.is_empty()
			|| identity.purpose != "atunnel"
		{
			return Err(ProxyError::SubstrateEgressDenied(
				"invalid ActorIdentity".to_owned(),
			));
		}
		Ok(identity)
	}

	pub(crate) async fn authorize_connect(
		&self,
		inputs: &Arc<ProxyInputs>,
		connection: &Extension,
		req: &mut Request,
	) -> Result<(), ProxyResponse> {
		let tcp = connection
			.copy::<TCPConnectionInfo>(req.extensions_mut())
			.expect("tcp connection must be set")
			.clone();
		connection.copy::<TLSConnectionInfo>(req.extensions_mut());
		let mut log = RequestLog::new(
			log::CelLogging::new(inputs.cfg.logging.clone(), inputs.cfg.metrics.clone()),
			inputs.metrics.clone(),
			inputs.model_catalog.clone(),
			agent_core::Timestamp::now(),
			tcp,
		);
		self
			.authorize(
				&PolicyClient::new(inputs.clone()).with_parent(req),
				&mut log,
				req,
			)
			.await
	}

	async fn authorize(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<(), ProxyResponse> {
		let identity = Self::identity(req)?;
		let actor = ActorRef {
			atespace: identity.atespace,
			name: identity.actor_name,
		};
		log.ate_actor_id = Some(actor.name.clone());
		log.ate_atespace = Some(actor.atespace.clone());
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
		if current
			.metadata
			.as_ref()
			.map(|metadata| metadata.uid.as_str())
			!= Some(identity.actor_uid.as_str())
		{
			return Err(ProxyError::SubstrateEgressDenied("actor UID mismatch".to_owned()).into());
		}
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
	use rcgen::{CertificateParams, CustomExtension, KeyPair};

	use super::*;
	use crate::http::Body;
	use crate::transport::tls::TlsInfo;

	fn request_with_identity(identity: &str) -> Request {
		let mut params = CertificateParams::default();
		params
			.custom_extensions
			.push(CustomExtension::from_oid_content(
				&[1, 3, 6, 1, 4, 1, 11129, 2, 12, 2],
				identity.as_bytes().to_vec(),
			));
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
		let identity = SubstrateEgress::identity(&request_with_identity(
			r#"{"Atespace":"demo","ActorName":"my-actor","ActorUid":"uid-1","Purpose":"atunnel"}"#,
		))
		.unwrap();
		assert_eq!(identity.atespace, "demo");
		assert_eq!(identity.actor_name, "my-actor");
		assert_eq!(identity.actor_uid, "uid-1");
	}

	#[test]
	fn actor_identity_requires_every_field_and_atunnel_purpose() {
		for identity in [
			r#"{"Atespace":"","ActorName":"my-actor","ActorUid":"uid-1","Purpose":"atunnel"}"#,
			r#"{"Atespace":"demo","ActorName":"","ActorUid":"uid-1","Purpose":"atunnel"}"#,
			r#"{"Atespace":"demo","ActorName":"my-actor","ActorUid":"","Purpose":"atunnel"}"#,
			r#"{"Atespace":"demo","ActorName":"my-actor","ActorUid":"uid-1","Purpose":"other"}"#,
		] {
			assert!(SubstrateEgress::identity(&request_with_identity(identity)).is_err());
		}
	}
}
