use tonic::Code;
use x509_parser::extensions::GeneralName;

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
	// Supplied by GetActor for logging; the certificate identifies the actor by name.
	pub(crate) actor_uid: Option<String>,
}

/// Validates an actor's identity before accepting a CONNECT tunnel.
#[apply(schema!)]
pub struct EgressActorResolution {
	/// Backend that receives GetActor calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
}

impl EgressActorResolution {
	fn identity(req: &Request) -> Result<ActorRef, ProxyError> {
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
		// TLS already authenticated this certificate. Require the ateom identity here:
		// a certificate issued directly to an actor must not authorize a tunnel.
		if certificate.is_ca() {
			return Err(ProxyError::SubstrateEgressDenied(
				"actor certificate is a CA certificate".to_owned(),
			));
		}
		let invalid_identity =
			|| ProxyError::SubstrateEgressDenied("invalid ateom-for-actor SPIFFE identity".to_owned());
		let sans = certificate
			.subject_alternative_name()
			.map_err(|_| invalid_identity())?
			.ok_or_else(invalid_identity)?;
		let mut uris = sans
			.value
			.general_names
			.iter()
			.filter_map(|name| match name {
				GeneralName::URI(uri) => Some(*uri),
				_ => None,
			});
		let uri = uris.next().ok_or_else(invalid_identity)?;
		if uris.next().is_some() {
			return Err(ProxyError::SubstrateEgressDenied(
				"actor certificate has multiple URI SANs".to_owned(),
			));
		}
		// Matches Substrate's resources.ActorRefFromAteomForActorSPIFFEID.
		let (atespace, name) = uri
			.strip_prefix("spiffe://substrate-actor.local/ateom-for-actor/")
			.and_then(|path| path.split_once('/'))
			.filter(|(atespace, name)| valid_resource_name(atespace) && valid_resource_name(name))
			.ok_or_else(invalid_identity)?;
		Ok(ActorRef {
			atespace: atespace.to_owned(),
			name: name.to_owned(),
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
		let actor = Self::identity(req)?;
		self
			.authorize(&PolicyClient::new(inputs.clone()).with_parent(req), &actor)
			.await
	}

	async fn authorize(
		&self,
		client: &PolicyClient,
		actor: &ActorRef,
	) -> Result<ActorIdentity, ProxyResponse> {
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
		Ok(ActorIdentity {
			atespace: actor.atespace.clone(),
			actor_name: actor.name.clone(),
			actor_uid: current
				.metadata
				.map(|metadata| metadata.uid)
				.filter(|uid| !uid.is_empty()),
		})
	}
}

#[cfg(test)]
mod tests {
	use rcgen::{CertificateParams, CustomExtension, KeyPair, SanType};

	use super::*;
	use crate::http::Body;
	use crate::transport::tls::TlsInfo;

	fn request_with_identity(uris: &[&str]) -> Request {
		let mut params = CertificateParams::default();
		params.subject_alt_names = uris
			.iter()
			.map(|uri| SanType::URI((*uri).try_into().unwrap()))
			.collect();
		request_with_certificate(params)
	}

	fn request_with_certificate(params: CertificateParams) -> Request {
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
	fn actor_identity_is_parsed_from_the_ateom_uri_without_an_extension() {
		for (atespace, name) in [
			("demo", "my-actor"),
			(
				"fd6cab8c-17c8-4c9e-8893-28e28aff724b",
				"045841a7-5dcb-47eb-a76f-6d8460bfe009",
			),
		] {
			let uri = format!("spiffe://substrate-actor.local/ateom-for-actor/{atespace}/{name}");
			let identity = EgressActorResolution::identity(&request_with_identity(&[&uri])).unwrap();
			assert_eq!(identity.atespace, atespace);
			assert_eq!(identity.name, name);
		}
	}

	#[test]
	fn actor_identity_rejects_other_purposes_and_malformed_uris() {
		for uri in [
			"spiffe://substrate-actor.local/actor/demo/my-actor",
			"spiffe://substrate-actor.local/atespace/demo/actor/my-actor",
			"spiffe://other.local/ateom-for-actor/demo/my-actor",
			"https://substrate-actor.local/ateom-for-actor/demo/my-actor",
			"spiffe://user@substrate-actor.local/ateom-for-actor/demo/my-actor",
			"spiffe://substrate-actor.local:443/ateom-for-actor/demo/my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor?query",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor#fragment",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor/",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor/extra",
			"spiffe://substrate-actor.local/ateom-for-actor//my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/",
			"spiffe://substrate-actor.local/ateom-for-actor/demo",
			"spiffe://substrate-actor.local/ateom-for-actor/de%2Fmo/my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my%2Factor",
			"spiffe://substrate-actor.local/ateom-for-actor/../my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/..",
			"spiffe://substrate-actor.local/ateom-for-actor/Demo/my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/My-Actor",
			"spiffe://substrate-actor.local/ateom-for-actor/-demo/my-actor",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor-",
		] {
			assert!(
				EgressActorResolution::identity(&request_with_identity(&[uri])).is_err(),
				"{uri}"
			);
		}
	}

	#[test]
	fn actor_identity_requires_exactly_one_uri_san() {
		let uri = "spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor";
		for uris in [vec![], vec![uri, uri], vec![uri, "https://example.com"]] {
			assert!(EgressActorResolution::identity(&request_with_identity(&uris)).is_err());
		}
		let params = CertificateParams::new(vec!["example.com".to_owned()]).unwrap();
		assert!(EgressActorResolution::identity(&request_with_certificate(params)).is_err());
	}

	#[test]
	fn actor_identity_rejects_ca_certificates() {
		let mut params = CertificateParams::default();
		params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
		params.subject_alt_names.push(SanType::URI(
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor"
				.try_into()
				.unwrap(),
		));
		assert!(EgressActorResolution::identity(&request_with_certificate(params)).is_err());
	}

	#[test]
	fn legacy_extension_cannot_authorize_a_tunnel() {
		let mut params = CertificateParams::default();
		params
			.custom_extensions
			.push(CustomExtension::from_oid_content(
				&[1, 3, 6, 1, 4, 1, 11129, 2, 12, 2],
				br#"{"Atespace":"demo","ActorName":"my-actor","ActorUid":"uid-1","Purpose":"atunnel"}"#
					.to_vec(),
			));
		assert!(EgressActorResolution::identity(&request_with_certificate(params)).is_err());
	}

	#[test]
	fn actor_identity_requires_an_authenticated_certificate() {
		let mut req = Request::new(Body::empty());
		req.headers_mut().insert(
			"x-forwarded-client-cert",
			"spiffe://substrate-actor.local/ateom-for-actor/demo/my-actor"
				.parse()
				.unwrap(),
		);
		assert!(EgressActorResolution::identity(&req).is_err());
		req.extensions_mut().insert(TLSConnectionInfo::default());
		assert!(EgressActorResolution::identity(&req).is_err());
	}
}
