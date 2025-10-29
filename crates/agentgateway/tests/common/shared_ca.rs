// Shared CA for test certificate generation
// Inspired by ztunnel's identity management approach for mTLS testing:
// https://github.com/istio/ztunnel/blob/master/src/tls/mock.rs
// https://github.com/istio/ztunnel/blob/master/src/test_helpers/ca.rs

use openssl::asn1::Asn1Time;
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::x509::X509;
use openssl::x509::X509Builder;
use openssl::x509::extension::BasicConstraints;
use std::sync::{Arc, OnceLock};

static SHARED_CA: OnceLock<SharedCA> = OnceLock::new();

#[derive(Clone)]
pub struct SharedCA {
	pub ca_key: Arc<PKey<Private>>,
	pub ca_cert: Arc<X509>,
}

impl SharedCA {
	fn new() -> anyhow::Result<Self> {
		let ca_key = PKey::from_rsa(Rsa::generate(2048)?)?;

		let mut ca_builder = X509Builder::new()?;
		ca_builder.set_version(2)?;

		let serial_number = {
			let mut serial = BigNum::new()?;
			serial.rand(159, openssl::bn::MsbOption::MAYBE_ZERO, false)?;
			serial.to_asn1_integer()?
		};
		ca_builder.set_serial_number(&serial_number)?;

		let mut ca_name = openssl::x509::X509NameBuilder::new()?;
		ca_name.append_entry_by_nid(Nid::COMMONNAME, "Test CA")?;
		ca_name.append_entry_by_nid(Nid::ORGANIZATIONNAME, "cluster.local")?;
		let ca_name = ca_name.build();

		ca_builder.set_subject_name(&ca_name)?;
		ca_builder.set_issuer_name(&ca_name)?;

		let not_before = Asn1Time::days_from_now(0)?;
		let not_after = Asn1Time::days_from_now(365)?;
		ca_builder.set_not_before(&not_before)?;
		ca_builder.set_not_after(&not_after)?;

		ca_builder.set_pubkey(&ca_key)?;

		let basic_constraints = BasicConstraints::new().critical().ca().build()?;
		ca_builder.append_extension(basic_constraints)?;

		ca_builder.sign(&ca_key, MessageDigest::sha256())?;
		let ca_cert = ca_builder.build();

		Ok(Self {
			ca_key: Arc::new(ca_key),
			ca_cert: Arc::new(ca_cert),
		})
	}
}

pub fn get_shared_ca() -> &'static SharedCA {
	SHARED_CA.get_or_init(|| SharedCA::new().expect("Failed to create shared CA"))
}
