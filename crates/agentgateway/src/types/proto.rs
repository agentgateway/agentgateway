use std::net;

use http::{status, uri};
use thiserror::Error;

#[allow(warnings)]
#[warn(clippy::derive_partial_eq_without_eq)]
pub mod workload {
	tonic::include_proto!("agentgateway.dev.workload");
}
#[allow(warnings)]
#[warn(clippy::derive_partial_eq_without_eq)]
pub mod agent {
	tonic::include_proto!("agentgateway.dev.resource");
}

#[allow(clippy::enum_variant_names)]
#[derive(Error, Debug)]
pub enum ProtoError {
	#[error("failed to parse namespaced hostname: {0}")]
	NamespacedHostnameParse(String),
	#[error("failed to parse address: {0}")]
	AddressParse(#[from] net::AddrParseError),
	#[error("failed to parse address, had {0} bytes")]
	ByteAddressParse(usize),
	#[error("invalid cidr: {0}")]
	PrefixParse(#[from] ipnet::PrefixLenError),
	#[error("unknown enum: {0}")]
	EnumParse(String),
	#[error("nonempty gateway address is missing address")]
	MissingGatewayAddress,
	#[error("decode error: {0}")]
	DecodeError(#[from] prost::DecodeError),
	#[error("decode error: {0}")]
	EnumError(#[from] prost::UnknownEnumValue),
	#[error("invalid URI: {0}")]
	InvalidURI(#[from] uri::InvalidUri),
	#[error("invalid status code: {0}")]
	InvalidStatusCode(#[from] status::InvalidStatusCode),
	#[error("error: {0}")]
	Generic(String),
	#[error("invalid header value: {0}")]
	HeaderValue(#[from] ::http::header::InvalidHeaderValue),
	#[error("invalid header name: {0}")]
	HeaderName(#[from] ::http::header::InvalidHeaderName),
	#[error("invalid regex: {0}")]
	Regex(#[from] regex::Error),
	#[error("invalid duration: {0}")]
	Duration(#[from] prost_types::DurationError),
	#[error("missing required field")]
	MissingRequiredField,
	#[error("invalid json: {0}")]
	Json(#[from] serde_json::Error),
}
