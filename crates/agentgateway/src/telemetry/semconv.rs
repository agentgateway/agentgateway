use http::Version;

pub(crate) mod attribute {
	pub const CLIENT_ADDRESS: &str = "client.address";
	pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
	pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
	pub const NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
	pub const SERVER_ADDRESS: &str = "server.address";
	pub const SERVER_PORT: &str = "server.port";
	pub const URL_PATH: &str = "url.path";
	pub const URL_QUERY: &str = "url.query";
	pub const URL_SCHEME: &str = "url.scheme";
}

pub(crate) fn http_attribute(key: &str) -> &str {
	match key {
		"src.addr" => attribute::CLIENT_ADDRESS,
		"http.method" => attribute::HTTP_REQUEST_METHOD,
		"http.host" => attribute::SERVER_ADDRESS,
		"http.path" => attribute::URL_PATH,
		"http.version" => attribute::NETWORK_PROTOCOL_VERSION,
		"http.status" => attribute::HTTP_RESPONSE_STATUS_CODE,
		key => key,
	}
}

pub(crate) fn protocol_version(version: &Version) -> Option<&'static str> {
	match *version {
		Version::HTTP_09 => Some("0.9"),
		Version::HTTP_10 => Some("1.0"),
		Version::HTTP_11 => Some("1.1"),
		Version::HTTP_2 => Some("2"),
		Version::HTTP_3 => Some("3"),
		_ => None,
	}
}

pub(crate) fn path_and_query(value: &str) -> (&str, Option<&str>) {
	value
		.split_once('?')
		.map_or((value, None), |(path, query)| (path, Some(query)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn converts_http_values() {
		assert_eq!(http_attribute("src.addr"), "client.address");
		assert_eq!(http_attribute("http.method"), "http.request.method");
		assert_eq!(http_attribute("http.host"), "server.address");
		assert_eq!(http_attribute("http.path"), "url.path");
		assert_eq!(http_attribute("http.version"), "network.protocol.version");
		assert_eq!(http_attribute("http.status"), "http.response.status_code");
		assert_eq!(http_attribute("custom"), "custom");

		assert_eq!(protocol_version(&Version::HTTP_09), Some("0.9"));
		assert_eq!(protocol_version(&Version::HTTP_10), Some("1.0"));
		assert_eq!(protocol_version(&Version::HTTP_11), Some("1.1"));
		assert_eq!(protocol_version(&Version::HTTP_2), Some("2"));
		assert_eq!(protocol_version(&Version::HTTP_3), Some("3"));
		assert_eq!(path_and_query("/get?q=otel"), ("/get", Some("q=otel")));
	}
}
