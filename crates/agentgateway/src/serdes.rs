use std::path::PathBuf;

pub use agent_core::serdes::*;
use openapiv3::OpenAPI;
use serde::de::DeserializeOwned;

use crate::resource_manager::{ResourceFetcher, ResourceKind, ResourceRef};

define_schema_aliases!();

/// Optional HTTP CONNECT / absolute-form proxy for a remote resource URL.
///
/// Mirrors backend `backendTunnel` semantics for control-plane style fetches
/// (JWKS, OIDC discovery, remote OpenAPI schemas). Only inline `host:port`
/// proxies are supported; named backends are out of scope for resource fetches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct RemoteHttpTunnel {
	/// Proxy used to reach the remote URL.
	pub proxy: RemoteHttpTunnelProxy,
}

/// Inline proxy address for [`RemoteHttpTunnel`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct RemoteHttpTunnelProxy {
	/// Proxy address as `host:port` (for example `corporate-proxy.example.com:8080`).
	pub host: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum FileInlineOrRemote {
	File {
		/// Path to a file on disk to load the value from.
		file: PathBuf,
	},
	Inline(String),
	Remote {
		#[serde(deserialize_with = "de_parse")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		url: http::Uri,
		/// Optional HTTP CONNECT / absolute-form proxy for this remote URL.
		/// When set, the fetch is tunneled through the proxy (same transport as
		/// backend `backendTunnel`). Does not honor `HTTP_PROXY` / `HTTPS_PROXY`.
		#[serde(default)]
		#[cfg_attr(feature = "schema", schemars(default))]
		tunnel: Option<RemoteHttpTunnel>,
	},
}

impl FileInlineOrRemote {
	pub async fn load<T: DeserializeOwned>(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<T> {
		let s = self.load_string(resources, kind).await?;
		serde_json::from_str(&s).map_err(Into::into)
	}

	pub async fn load_openapi_schema(&self, resources: &ResourceFetcher) -> anyhow::Result<OpenAPI> {
		let s = self.load_string(resources, ResourceKind::OpenApi).await?;
		stacker::grow(2 * 1024 * 1024, || {
			yamlviajson::from_str::<OpenAPI>(s.as_str())
		})
	}

	async fn load_string(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<String> {
		Ok(match self {
			FileInlineOrRemote::Inline(s) => s.clone(),
			FileInlineOrRemote::File { .. } | FileInlineOrRemote::Remote { .. } => {
				let bytes = resources
					.fetch(self.as_resource_ref(kind).expect("resource ref"))
					.await?;
				String::from_utf8(bytes.to_vec())?
			},
		})
	}

	fn as_resource_ref(&self, kind: ResourceKind) -> Option<ResourceRef> {
		match self {
			FileInlineOrRemote::File { file } => Some(ResourceRef::File(file.clone())),
			FileInlineOrRemote::Inline(_) => None,
			FileInlineOrRemote::Remote { url, tunnel } => Some(ResourceRef::Http {
				url: url.clone(),
				kind,
				tunnel: tunnel.as_ref().map(|t| t.proxy.host.clone()),
			}),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn remote_without_tunnel_deserializes() {
		let v: FileInlineOrRemote = serde_json::from_value(serde_json::json!({
			"url": "https://idp.example.com/.well-known/jwks.json"
		}))
		.unwrap();
		match v {
			FileInlineOrRemote::Remote { url, tunnel } => {
				assert_eq!(
					url.to_string(),
					"https://idp.example.com/.well-known/jwks.json"
				);
				assert!(tunnel.is_none());
			},
			other => panic!("expected Remote, got {other:?}"),
		}
	}

	#[test]
	fn remote_with_tunnel_deserializes() {
		let v: FileInlineOrRemote = serde_json::from_value(serde_json::json!({
			"url": "https://idp.example.com/.well-known/jwks.json",
			"tunnel": {
				"proxy": {
					"host": "corporate-proxy.example.com:8080"
				}
			}
		}))
		.unwrap();
		match v {
			FileInlineOrRemote::Remote { url, tunnel } => {
				assert_eq!(
					url.to_string(),
					"https://idp.example.com/.well-known/jwks.json"
				);
				let tunnel = tunnel.expect("tunnel");
				assert_eq!(tunnel.proxy.host, "corporate-proxy.example.com:8080");
			},
			other => panic!("expected Remote, got {other:?}"),
		}
	}
}
