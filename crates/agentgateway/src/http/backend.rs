use serde::{Deserialize, Serialize};

/// HTTP version preference for upstream backend connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HTTP {
    /// Optional HTTP version override for this backend.
    /// When set to "1.1", forces HTTP/1.1 and (when TLS is configured) restricts ALPN to http/1.1.
    /// When set to "2", prefers HTTP/2 (best-effort via ALPN over TLS).
    #[serde(default)]
    pub version: Option<HttpVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum HttpVersion {
    #[serde(rename = "1.1")]
    Http1_1,
    #[serde(rename = "2")]
    Http2,
}

impl HTTP {
    pub fn is_http11(&self) -> bool {
        matches!(self.version, Some(HttpVersion::Http1_1))
    }
    pub fn is_http2(&self) -> bool {
        matches!(self.version, Some(HttpVersion::Http2))
    }
    pub fn version_override(&self) -> Option<http::Version> {
        match self.version {
            Some(HttpVersion::Http1_1) => Some(http::Version::HTTP_11),
            Some(HttpVersion::Http2) => Some(http::Version::HTTP_2),
            None => None,
        }
    }
}

