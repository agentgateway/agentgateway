use crate::proto;
use crate::proto::agentgateway::dev::a2a::target::Target as XdsA2aTarget;
use crate::proto::agentgateway::dev::common::BackendAuth as XdsAuth;
use crate::proto::agentgateway::dev::common::BackendTls as XdsTls;
use crate::proto::agentgateway::dev::mcp::target::{
	Target as McpXdsTarget, target::Filter as XdsFilter, target::OpenApiTarget as XdsOpenAPITarget,
	target::SseTarget as XdsSseTarget, target::Target as XdsTarget,
	target::filter::Matcher as XdsFitlerMatcher,
};
use crate::relay::{Filter, FilterMatcher};
use openapiv3::OpenAPI;
use rmcp::model::Tool;
use serde::Serialize;
use std::collections::HashMap;
pub mod backend;
pub mod openapi;

use {once_cell::sync::Lazy, regex::Regex};

const VALID_NAME_REGEX: &str = r"^[a-zA-Z0-9-]+$";

fn is_valid_name(name: &str) -> bool {
	// We cannot support underscores in the name because they are used to separate the name from the listener name.
	static RE: Lazy<Regex> = Lazy::new(|| Regex::new(VALID_NAME_REGEX).unwrap());
	RE.is_match(name)
}

#[derive(Clone, Serialize, Debug)]
pub struct Target<T> {
	pub name: String,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub listeners: Vec<String>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<Filter>,
	pub spec: T,
}

impl TryFrom<McpXdsTarget> for Target<McpTargetSpec> {
	type Error = anyhow::Error;

	fn try_from(value: McpXdsTarget) -> Result<Self, Self::Error> {
		let target = match value.target {
			Some(target) => target,
			None => return Err(anyhow::anyhow!("target is None")),
		};

		if !is_valid_name(&value.name) {
			return Err(anyhow::anyhow!(
				"invalid name: {}, must match regex: {}",
				value.name,
				VALID_NAME_REGEX
			));
		}

		Ok(Target {
			name: value.name,
			listeners: value.listeners,
			filters: value
				.filters
				.into_iter()
				.map(|f| f.try_into())
				.collect::<Result<Vec<_>, _>>()?,
			spec: target.try_into()?,
		})
	}
}

impl TryFrom<XdsFilter> for Filter {
	type Error = anyhow::Error;

	fn try_from(value: XdsFilter) -> Result<Self, Self::Error> {
		let matcher = match XdsFitlerMatcher::try_from(value.matcher)? {
			XdsFitlerMatcher::Equals => FilterMatcher::Equals(value.r#match.clone()),
			XdsFitlerMatcher::Prefix => FilterMatcher::Prefix(value.r#match.clone()),
			XdsFitlerMatcher::Suffix => FilterMatcher::Suffix(value.r#match.clone()),
			XdsFitlerMatcher::Contains => FilterMatcher::Contains(value.r#match.clone()),
			XdsFitlerMatcher::Regex => FilterMatcher::Regex(Regex::new(&value.r#match)?),
		};
		Ok(Filter::new(matcher, value.r#type))
	}
}

#[derive(Clone, Serialize, Debug)]
pub enum McpTargetSpec {
	Sse(SseTargetSpec),
	Stdio {
		cmd: String,
		#[serde(skip_serializing_if = "Vec::is_empty")]
		args: Vec<String>,
		#[serde(skip_serializing_if = "HashMap::is_empty")]
		env: HashMap<String, String>,
	},
	OpenAPI(OpenAPITarget),
}

impl TryFrom<XdsTarget> for McpTargetSpec {
	type Error = anyhow::Error;

	fn try_from(value: XdsTarget) -> Result<Self, Self::Error> {
		let target = match value {
			XdsTarget::Sse(sse) => McpTargetSpec::Sse(sse.try_into()?),
			XdsTarget::Stdio(stdio) => McpTargetSpec::Stdio {
				cmd: stdio.cmd,
				args: stdio.args,
				env: stdio.env,
			},
			XdsTarget::Openapi(openapi) => McpTargetSpec::OpenAPI(openapi.try_into()?),
		};
		Ok(target)
	}
}

#[derive(Clone, Serialize, Debug)]
pub enum A2aTargetSpec {
	Sse(SseTargetSpec),
}

impl TryFrom<XdsA2aTarget> for Target<A2aTargetSpec> {
	type Error = anyhow::Error;

	fn try_from(value: XdsA2aTarget) -> Result<Self, Self::Error> {
		if !is_valid_name(&value.name) {
			return Err(anyhow::anyhow!(
				"invalid name: {}, must match regex: {}",
				value.name,
				VALID_NAME_REGEX
			));
		}
		Ok(Target {
			name: value.name,
			listeners: value.listeners,
			filters: vec![], // TODO: Add filters
			spec: A2aTargetSpec::Sse(SseTargetSpec {
				host: value.host,
				port: value.port,
				path: value.path,
				headers: proto::resolve_header_map(&value.headers)?,
				backend_auth: match value.auth {
					Some(auth) => XdsAuth::try_into(auth)?,
					None => None,
				},
				tls: match value.tls {
					Some(tls) => Some(TlsConfig::try_from(tls)?),
					None => None,
				},
			}),
		})
	}
}

#[derive(Clone, Serialize, Debug)]
pub struct SseTargetSpec {
	pub host: String,
	pub port: u32,
	pub path: String,
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub headers: HashMap<String, String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub backend_auth: Option<backend::BackendAuthConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tls: Option<TlsConfig>,
}

#[derive(Clone, Serialize, Debug)]
pub struct TlsConfig {
	pub insecure_skip_verify: bool,
}

impl TryFrom<XdsTls> for TlsConfig {
	type Error = anyhow::Error;

	fn try_from(value: XdsTls) -> Result<Self, Self::Error> {
		Ok(TlsConfig {
			insecure_skip_verify: value.insecure_skip_verify,
		})
	}
}

impl TryFrom<XdsSseTarget> for SseTargetSpec {
	type Error = anyhow::Error;

	fn try_from(value: XdsSseTarget) -> Result<Self, Self::Error> {
		Ok(SseTargetSpec {
			host: value.host,
			port: value.port,
			path: value.path,
			headers: proto::resolve_header_map(&value.headers)?,
			backend_auth: match value.auth {
				Some(auth) => XdsAuth::try_into(auth)?,
				None => None,
			},
			tls: match value.tls {
				Some(tls) => Some(TlsConfig::try_from(tls)?),
				None => None,
			},
		})
	}
}

#[derive(Clone, Serialize, Debug)]
pub struct OpenAPITarget {
	pub host: String,
	pub prefix: String,
	pub port: u32,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub tools: Vec<(Tool, openapi::UpstreamOpenAPICall)>,
	#[serde(skip_serializing_if = "HashMap::is_empty")]
	pub headers: HashMap<String, String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub backend_auth: Option<backend::BackendAuthConfig>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tls: Option<TlsConfig>,
}

impl TryFrom<XdsOpenAPITarget> for OpenAPITarget {
	type Error = openapi::ParseError;

	fn try_from(value: XdsOpenAPITarget) -> Result<Self, Self::Error> {
		let schema = value.schema.ok_or(openapi::ParseError::MissingSchema)?;
		let schema_bytes =
			proto::resolve_local_data_source(&schema.source.ok_or(openapi::ParseError::MissingFields)?)?;
		let schema: OpenAPI =
			serde_json::from_slice(&schema_bytes).map_err(openapi::ParseError::SerdeError)?;
		let tools = openapi::parse_openapi_schema(&schema)?;
		let prefix = openapi::get_server_prefix(&schema)?;
		let headers = proto::resolve_header_map(&value.headers)?;
		Ok(OpenAPITarget {
			host: value.host.clone(),
			prefix,
			port: value.port,
			tools,
			headers,
			backend_auth: match value.auth {
				Some(auth) => auth
					.try_into()
					.map_err(|_| openapi::ParseError::MissingSchema)?,
				None => None,
			},
			tls: match value.tls {
				Some(tls) => {
					Some(TlsConfig::try_from(tls).map_err(|_| openapi::ParseError::MissingSchema)?)
				},
				None => None,
			},
		})
	}
}
