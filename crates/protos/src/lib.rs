#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod envoy {
	pub mod service {
		pub mod auth {
			pub mod v3 {
				tonic::include_proto!("envoy.service.auth.v3");
			}
		}

		pub mod common {
			pub mod v3 {
				tonic::include_proto!("envoy.service.common.v3");
			}
		}

		pub mod discovery {
			pub mod v3 {
				tonic::include_proto!("envoy.service.discovery.v3");
			}
		}

		pub mod ext_proc {
			pub mod v3 {
				tonic::include_proto!("envoy.service.ext_proc.v3");
			}
		}

		pub mod ratelimit {
			pub mod v3 {
				tonic::include_proto!("envoy.service.ratelimit.v3");
			}
		}
	}
}

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod istio {
	pub mod workload {
		tonic::include_proto!("istio.workload");
	}

	pub mod v1 {
		pub mod auth {
			tonic::include_proto!("istio.v1.auth");
		}
	}
}

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
mod agentgateway_internal {
	pub mod dev {
		pub mod resource {
			tonic::include_proto!("agentgateway.dev.resource");
		}
	}
}

pub mod agentgateway {
	pub mod dev {
		pub mod resource {
			pub use crate::agentgateway_internal::dev::resource::*;
		}
	}
}

pub mod agent {
	pub use crate::agentgateway::dev::resource::*;
}

pub mod workload {
	pub use crate::istio::workload::*;
}
