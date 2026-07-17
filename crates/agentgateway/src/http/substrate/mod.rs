mod egress;
mod ingress;

pub use egress::SubstrateEgress;
pub use ingress::SubstrateIngress;
pub(crate) use ingress::{SubstrateRequestState, is_stale_assignment};

const CACHE_CAPACITY: usize = 10_000;
const TRACE_POLICY_KIND: &str = "substrate";

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
