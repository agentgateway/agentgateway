// Shared CA for test certificate generation
// Uses static pre-generated CA keys for consistent test certificate generation
//
// To regenerate these test certificates, run:
//   cd crates/agentgateway/tests/common/testdata
//   ./gen_certs.sh

use std::sync::{Arc, OnceLock};

use rcgen::KeyPair;

// Static test CA files
pub const TEST_ROOT_KEY: &[u8] = include_bytes!("testdata/ca-key.pem");
pub const TEST_ROOT: &[u8] = include_bytes!("testdata/root-cert.pem");
pub const TEST_PKEY: &[u8] = include_bytes!("testdata/key.pem");

static SHARED_CA: OnceLock<SharedCA> = OnceLock::new();

#[derive(Clone)]
pub struct SharedCA {
	// Store the CA key pair for signing
	pub ca_key: Arc<KeyPair>,
	// Store the CA cert PEM for trust stores
	pub ca_cert_pem: Arc<String>,
}

impl SharedCA {
	fn new() -> anyhow::Result<Self> {
		// Load the pre-generated CA key
		let ca_key = KeyPair::from_pem(std::str::from_utf8(TEST_ROOT_KEY)?)?;
		let ca_cert_pem = String::from_utf8(TEST_ROOT.to_vec())?;

		Ok(Self {
			ca_key: Arc::new(ca_key),
			ca_cert_pem: Arc::new(ca_cert_pem),
		})
	}
}

pub fn get_shared_ca() -> &'static SharedCA {
	SHARED_CA.get_or_init(|| SharedCA::new().expect("Failed to create shared CA"))
}
