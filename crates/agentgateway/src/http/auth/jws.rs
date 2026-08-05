use std::fmt;

use anyhow::Context;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Serialize;

use crate::types::proto::agent as proto;
use crate::{apply, schema_enum};

#[apply(schema_enum!)]
#[derive(Default)]
pub enum JwtSigningAlg {
	#[default]
	#[serde(rename = "RS256")]
	Rs256,
	#[serde(rename = "RS384")]
	Rs384,
	#[serde(rename = "RS512")]
	Rs512,
	#[serde(rename = "PS256")]
	Ps256,
	#[serde(rename = "ES256")]
	Es256,
	#[serde(rename = "ES384")]
	Es384,
}

impl JwtSigningAlg {
	fn algorithm(self) -> Algorithm {
		match self {
			Self::Rs256 => Algorithm::RS256,
			Self::Rs384 => Algorithm::RS384,
			Self::Rs512 => Algorithm::RS512,
			Self::Ps256 => Algorithm::PS256,
			Self::Es256 => Algorithm::ES256,
			Self::Es384 => Algorithm::ES384,
		}
	}

	pub(crate) fn encoding_key(self, pem: &[u8]) -> anyhow::Result<EncodingKey> {
		match self {
			Self::Rs256 | Self::Rs384 | Self::Rs512 | Self::Ps256 => {
				EncodingKey::from_rsa_pem(pem).context("failed to load RSA signing key")
			},
			Self::Es256 | Self::Es384 => {
				EncodingKey::from_ec_pem(pem).context("failed to load EC signing key")
			},
		}
	}

	pub(crate) fn header(self, kid: Option<String>) -> Header {
		let mut header = Header::new(self.algorithm());
		header.kid = kid;
		header
	}
}

pub(crate) fn signing_alg_from_proto(alg: i32) -> Option<JwtSigningAlg> {
	use proto::JwtSigningAlg as ProtoSigningAlg;

	match ProtoSigningAlg::try_from(alg) {
		Ok(ProtoSigningAlg::Unspecified | ProtoSigningAlg::Rs256) => Some(JwtSigningAlg::Rs256),
		Ok(ProtoSigningAlg::Rs384) => Some(JwtSigningAlg::Rs384),
		Ok(ProtoSigningAlg::Rs512) => Some(JwtSigningAlg::Rs512),
		Ok(ProtoSigningAlg::Ps256) => Some(JwtSigningAlg::Ps256),
		Ok(ProtoSigningAlg::Es256) => Some(JwtSigningAlg::Es256),
		Ok(ProtoSigningAlg::Es384) => Some(JwtSigningAlg::Es384),
		Err(_) => None,
	}
}

#[derive(Clone)]
pub(crate) struct SigningKey(EncodingKey);

impl SigningKey {
	pub(crate) fn from_pem(alg: JwtSigningAlg, pem: &[u8]) -> anyhow::Result<Self> {
		alg.encoding_key(pem).map(Self)
	}

	pub(crate) fn encode<T: Serialize>(
		&self,
		header: &Header,
		claims: &T,
	) -> jsonwebtoken::errors::Result<String> {
		jsonwebtoken::encode(header, claims, &self.0)
	}
}

impl fmt::Debug for SigningKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("[REDACTED]")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn signing_alg_proto_mapping_defaults_and_rejects_unknown_values() {
		use proto::JwtSigningAlg as ProtoSigningAlg;

		let cases = [
			(ProtoSigningAlg::Unspecified, JwtSigningAlg::Rs256),
			(ProtoSigningAlg::Rs256, JwtSigningAlg::Rs256),
			(ProtoSigningAlg::Rs384, JwtSigningAlg::Rs384),
			(ProtoSigningAlg::Rs512, JwtSigningAlg::Rs512),
			(ProtoSigningAlg::Ps256, JwtSigningAlg::Ps256),
			(ProtoSigningAlg::Es256, JwtSigningAlg::Es256),
			(ProtoSigningAlg::Es384, JwtSigningAlg::Es384),
		];

		for (proto, expected) in cases {
			assert_eq!(
				signing_alg_from_proto(proto as i32).expect("known algorithm should map"),
				expected
			);
		}
		assert_eq!(signing_alg_from_proto(i32::MAX), None);
	}
}
