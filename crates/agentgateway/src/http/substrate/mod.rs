pub(crate) mod ateattr;
mod egress;
mod egress_actor_resolution;
mod ingress;

pub use egress::SubstrateEgress;
pub(crate) use egress::{EgressRequestProtocol, EgressTlsMode, authorize_tls, reject_upgrade};
pub(crate) use egress_actor_resolution::ActorIdentity;
pub use egress_actor_resolution::EgressActorResolution;
pub use ingress::SubstrateIngress;
pub(crate) use ingress::{
	STALE_ASSIGNMENT_HEADER, SubstrateRequestState, is_stale_assignment, stale_assignment_unavailable,
};

use tonic::Status;
use tonic_types::{ErrorDetail, StatusExt};

use crate::proxy::SubstrateEgressError;

const CACHE_CAPACITY: usize = 10_000;
const TRACE_POLICY_KIND: &str = "substrate";

fn status_reason(status: &Status) -> Option<String> {
	status
		.get_error_details_vec()
		.into_iter()
		.find_map(|detail| match detail {
			ErrorDetail::ErrorInfo(info) if info.domain == "ate.dev" && !info.reason.is_empty() => {
				Some(info.reason)
			},
			_ => None,
		})
}

fn egress_error(message: String, status: &Status, fallback: &'static str) -> SubstrateEgressError {
	let reason = status_reason(status).filter(|reason| {
		(1..=64).contains(&reason.len())
			&& reason.as_bytes()[0].is_ascii_uppercase()
			&& reason
				.bytes()
				.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
	});
	SubstrateEgressError::new(message, reason.unwrap_or_else(|| fallback.to_owned()))
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ActorRef {
	atespace: String,
	name: String,
}

fn valid_resource_name(name: &str) -> bool {
	let bytes = name.as_bytes();
	(1..=63).contains(&bytes.len())
		&& bytes.first().is_some_and(u8::is_ascii_alphanumeric)
		&& bytes.last().is_some_and(u8::is_ascii_alphanumeric)
		&& bytes
			.iter()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

#[cfg(test)]
mod tests {
	use tonic::Code;
	use tonic_types::ErrorDetails;

	use super::*;
	use crate::proxy::ProxyError;

	#[test]
	fn egress_reason_uses_trusted_error_info() {
		let status = Status::with_error_details(
			Code::PermissionDenied,
			"provider refused",
			ErrorDetails::with_error_info(
				"ACCOUNT_CONNECTION_REQUIRED",
				"ate.dev",
				std::collections::HashMap::new(),
			),
		);
		let response = ProxyError::SubstrateEgressDenied(egress_error(
			"provider refused".to_owned(),
			&status,
			"CREDENTIAL_PROVIDER_DENIED",
		))
		.into_response_with_grpc(false);
		assert_eq!(response.status(), ::http::StatusCode::FORBIDDEN);
		assert_eq!(
			response.headers()["ate-error-reason"],
			"ACCOUNT_CONNECTION_REQUIRED"
		);
	}

	#[test]
	fn egress_reason_rejects_untrusted_or_invalid_error_info() {
		for (reason, domain) in [
			("ACCOUNT_CONNECTION_REQUIRED", "other.dev"),
			("lower_case", "ate.dev"),
			("BAD\nHEADER", "ate.dev"),
		] {
			let status = Status::with_error_details(
				Code::PermissionDenied,
				"provider refused",
				ErrorDetails::with_error_info(reason, domain, std::collections::HashMap::new()),
			);
			let response = ProxyError::SubstrateEgressDenied(egress_error(
				"provider refused".to_owned(),
				&status,
				"CREDENTIAL_PROVIDER_DENIED",
			))
			.into_response_with_grpc(false);
			assert_eq!(
				response.headers()["ate-error-reason"],
				"CREDENTIAL_PROVIDER_DENIED"
			);
		}
	}
}
