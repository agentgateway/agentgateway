use ::http::uri::PathAndQuery;
use percent_encoding::percent_decode_str;

use super::Error;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedirectUri {
	#[serde(rename = "redirectURI")]
	pub redirect_uri: String,
	pub host: String,
	pub port: u16,
	pub https: bool,
	#[serde(
		serialize_with = "crate::serdes::ser_display",
		deserialize_with = "crate::serdes::de_parse"
	)]
	pub callback_path: PathAndQuery,
}

impl RedirectUri {
	pub fn parse(redirect_uri: String) -> Result<Self, Error> {
		let http_uri = redirect_uri
			.parse::<::http::Uri>()
			.map_err(|e| Error::Config(format!("invalid redirectURI: {e}")))?;
		let url = url::Url::parse(&redirect_uri)
			.map_err(|e| Error::Config(format!("invalid redirectURI: {e}")))?;
		if !url.username().is_empty() || url.password().is_some() {
			return Err(Error::Config(
				"redirectURI must not contain userinfo".into(),
			));
		}
		if url.fragment().is_some() {
			return Err(Error::Config(
				"redirectURI must not contain a fragment".into(),
			));
		}
		if url.query().is_some() {
			return Err(Error::Config(
				"redirectURI must not contain query parameters".into(),
			));
		}

		let https = match url.scheme() {
			"https" => true,
			"http" => false,
			other => {
				return Err(Error::Config(format!(
					"redirectURI must use http or https, got scheme '{other}'"
				)));
			},
		};
		let host = url
			.host_str()
			.ok_or_else(|| Error::Config("redirectURI must include a host".into()))?
			.to_ascii_lowercase();
		let port = url
			.port_or_known_default()
			.ok_or_else(|| Error::Config("redirectURI must include an http or https authority".into()))?;
		let callback_path = normalize_callback_path(http_uri.path())?;

		Ok(Self {
			redirect_uri,
			host,
			port,
			https,
			callback_path,
		})
	}

	pub fn canonical_uri(&self) -> String {
		let scheme = if self.https { "https" } else { "http" };
		let host = if self.host.contains(':') {
			format!("[{}]", self.host)
		} else {
			self.host.clone()
		};
		format!("{scheme}://{host}:{}{}", self.port, self.callback_path)
	}
}

fn normalize_callback_path(path: &str) -> Result<PathAndQuery, Error> {
	if path.is_empty() || path == "/" {
		return Err(Error::Config(
			"redirectURI path must be explicit and non-root".into(),
		));
	}
	if !path.starts_with('/') {
		return Err(Error::Config("redirectURI path must start with '/'".into()));
	}
	// Reject dot segments and encoded slash/backslash forms so callback ownership stays exact and
	// does not depend on downstream path normalization behavior.
	for segment in path.split('/').skip(1) {
		let decoded = percent_decode_str(segment)
			.decode_utf8()
			.map_err(|_| Error::Config("redirectURI path contains invalid percent-encoding".into()))?;
		if decoded == "." || decoded == ".." {
			return Err(Error::Config(
				"redirectURI path must not contain dot segments".into(),
			));
		}
		if decoded.contains('/') || decoded.contains('\\') {
			return Err(Error::Config(
				"redirectURI path must not contain encoded slash or backslash characters".into(),
			));
		}
	}
	PathAndQuery::try_from(path).map_err(|e| Error::Config(format!("invalid redirectURI path: {e}")))
}
