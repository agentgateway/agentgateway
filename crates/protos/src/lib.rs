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

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod ext_mcp {
	tonic::include_proto!("agentgateway.dev.ext_mcp");
}

pub mod workload {
	pub use crate::istio::workload::*;
}

// SPIFFE Workload API bindings (gRPC client + server), generated from proto/spiffe_workload.proto.
//
// TEST/E2E ONLY: gated behind the `spiffe-test-server` feature, which is enabled via agentgateway's
// dev-dependencies.
// We only ever use the generated *server* (to stand up a fake Workload API in tests); the gateway's
// real client is the `spiffe` crate, talking to a real SPIFFE Workload API endpoint.
//
// Why `include_proto!("_")`: the macro argument is a protobuf *package* name, not a file name, and
// it expands to `include!(OUT_DIR/<package>.rs)`. spiffe_workload.proto declares no package, and
// prost names the empty package's output `_.rs` — hence `"_"`.
#[cfg(feature = "spiffe-test-server")]
#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod spiffe_workload_api {
	tonic::include_proto!("_");
}
