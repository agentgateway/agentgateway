use std::net::IpAddr;
use std::str::FromStr;

use ::http::{StatusCode, header};
use serde::de::Error;

use crate::http::{PolicyResponse, Request, filters};
use crate::*;

#[derive(Default, Debug, Clone)]
enum WildcardOrList<T> {
	#[default]
	None,
	Wildcard,
	List(Vec<T>),
}

impl<T> WildcardOrList<T> {
	fn is_none(&self) -> bool {
		matches!(self, WildcardOrList::None)
	}
}

impl<T: FromStr> TryFrom<Vec<String>> for WildcardOrList<T> {
	type Error = T::Err;

	fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
		if value.contains(&"*".to_string()) {
			Ok(WildcardOrList::Wildcard)
		} else if value.is_empty() {
			Ok(WildcardOrList::None)
		} else {
			let vec: Vec<T> = value
				.into_iter()
				.map(|v| T::from_str(&v))
				.collect::<Result<_, _>>()?;
			Ok(WildcardOrList::List(vec))
		}
	}
}

impl<T: Display> Serialize for WildcardOrList<T> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			WildcardOrList::None => Vec::<String>::new().serialize(serializer),
			WildcardOrList::Wildcard => vec!["*"].serialize(serializer),
			WildcardOrList::List(list) => list
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.serialize(serializer),
		}
	}
}

#[derive(Debug, Clone)]
enum IpRange {
	Single(IpAddr),
	Cidr(ipnet::IpNet),
}

impl IpRange {
	fn contains(&self, ip: IpAddr) -> bool {
		match self {
			IpRange::Single(allowed) => allowed == &ip,
			IpRange::Cidr(network) => network.contains(&ip),
		}
	}
}

impl FromStr for IpRange {
	type Err = anyhow::Error;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		// Try parsing as CIDR first
		if let Ok(network) = ipnet::IpNet::from_str(s) {
			return Ok(IpRange::Cidr(network));
		}
		// Otherwise try as single IP
		if let Ok(ip) = IpAddr::from_str(s) {
			return Ok(IpRange::Single(ip));
		}
		Err(anyhow::anyhow!("Invalid IP address or CIDR: {}", s))
	}
}

impl Display for IpRange {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			IpRange::Single(ip) => write!(f, "{}", ip),
			IpRange::Cidr(network) => write!(f, "{}", network),
		}
	}
}

/// Source from which to extract the client IP address
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum IpSource {
	/// Use the remote socket address from the direct connection
	RemoteAddr,
	/// Use the X-Forwarded-For header
	#[default]
	XForwardedFor,
}

#[apply(schema_ser!)]
#[cfg_attr(feature = "schema", schemars(with = "IpAllowlistSerde"))]
pub struct IpAllowlist {
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allowed_ranges: WildcardOrList<IpRange>,
	#[serde(with = "http_serde::status_code")]
	#[cfg_attr(feature = "schema", schemars(with = "std::num::NonZeroU16"))]
	deny_status: StatusCode,
	#[serde(serialize_with = "ser_string_or_bytes_option")]
	deny_message: Option<::http::HeaderValue>,
	ip_source: IpSource,
	/// Distance from the last hop in X-Forwarded-For header.
	/// 0 = last hop (rightmost), -1 = second to last, -2 = third to last, etc.
	/// If the list has fewer IPs than requested, uses the first IP.
	distance_from_last_hop: i32,
}

impl<'de> serde::Deserialize<'de> for IpAllowlist {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		IpAllowlist::try_from(IpAllowlistSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct IpAllowlistSerde {
	#[serde(default)]
	pub allowed_ips: Vec<String>,
	#[serde(default = "default_deny_status_code")]
	pub deny_status_code: u16,
	#[serde(default)]
	pub deny_message: Option<String>,
	#[serde(default)]
	pub ip_source: IpSource,
	#[serde(default)]
	pub distance_from_last_hop: i32,
}

fn default_deny_status_code() -> u16 {
	403
}

impl TryFrom<IpAllowlistSerde> for IpAllowlist {
	type Error = anyhow::Error;
	fn try_from(value: IpAllowlistSerde) -> Result<Self, Self::Error> {
		Ok(IpAllowlist {
			allowed_ranges: WildcardOrList::try_from(value.allowed_ips)?,
			deny_status: StatusCode::from_u16(value.deny_status_code)?,
			deny_message: value
				.deny_message
				.map(|msg| http::HeaderValue::from_str(&msg))
				.transpose()?,
			ip_source: value.ip_source,
			distance_from_last_hop: value.distance_from_last_hop,
		})
	}
}

impl IpAllowlist {
	/// Apply the IP allowlist policy to the request.
	/// If the source IP is not in the allowlist, returns a direct response with the configured deny status.
	/// Otherwise, allows the request to continue processing.
	pub fn apply(&self, req: &mut Request) -> Result<PolicyResponse, filters::Error> {
		// Extract source IP from request
		let source_ip = self.extract_source_ip(req)?;

		// Check against allowlist
		let allowed = match &self.allowed_ranges {
			WildcardOrList::None => false,    // Empty list = deny all
			WildcardOrList::Wildcard => true, // "*" = allow all
			WildcardOrList::List(ranges) => ranges.iter().any(|range| range.contains(source_ip)),
		};

		// If not allowed, return direct deny response
		if !allowed {
			let body = self
				.deny_message
				.as_ref()
				.map(|hv| hv.as_bytes().to_vec())
				.unwrap_or_else(|| b"Forbidden: IP not allowed".to_vec());

			let response = ::http::Response::builder()
				.status(self.deny_status)
				.header(header::CONTENT_TYPE, "text/plain")
				.body(crate::http::Body::from(body))?;

			return Ok(PolicyResponse {
				direct_response: Some(response),
				response_headers: None,
			});
		}

		// If allowed, continue processing
		Ok(PolicyResponse::default())
	}

	/// Extract the source IP address from the request based on configuration.
	fn extract_source_ip(&self, req: &Request) -> Result<IpAddr, filters::Error> {
		match self.ip_source {
			IpSource::RemoteAddr => {
				// Use remote socket address
				if let Some(addr) = req.extensions().get::<std::net::SocketAddr>() {
					return Ok(addr.ip());
				}
				Err(filters::Error::InvalidFilterConfiguration(
					"Unable to determine remote socket address".to_string(),
				))
			},
			IpSource::XForwardedFor => {
				// Use X-Forwarded-For header
				if let Some(xff) = req.headers().get("X-Forwarded-For")
					&& let Ok(xff_str) = xff.to_str()
				{
					return self.extract_ip_from_xff(xff_str);
				}
				Err(filters::Error::InvalidFilterConfiguration(
					"Unable to determine IP from X-Forwarded-For header".to_string(),
				))
			},
		}
	}

	/// Extract IP from X-Forwarded-For header based on distance_from_last_hop.
	/// distanceFromLastHop: 0 = last hop (rightmost), -1 = second to last, etc.
	/// If the list has fewer IPs than requested, uses the first IP.
	fn extract_ip_from_xff(&self, xff_str: &str) -> Result<IpAddr, filters::Error> {
		let ips: Vec<&str> = xff_str.split(',').map(|s| s.trim()).collect();

		if ips.is_empty() {
			return Err(filters::Error::InvalidFilterConfiguration(
				"X-Forwarded-For header is empty".to_string(),
			));
		}

		// Calculate the index based on distance_from_last_hop
		// distance_from_last_hop = 0 means last (rightmost)
		// distance_from_last_hop = -1 means second to last
		// distance_from_last_hop = -2 means third to last, etc.
		let index = if self.distance_from_last_hop >= 0 {
			// Positive or zero: count from the end (0 = last)
			let idx = (ips.len() as i32 - 1 - self.distance_from_last_hop).max(0) as usize;
			idx.min(ips.len() - 1)
		} else {
			// Negative: count from the end (-1 = second to last, -2 = third to last, etc.)
			// -1 means 1 position before last, so we want len - 1 - 1 = len - 2
			let offset = (-self.distance_from_last_hop) as usize;
			if offset >= ips.len() {
				// If we go past the beginning, use the first IP
				0
			} else {
				ips.len() - 1 - offset
			}
		};

		ips[index].parse().map_err(|_| {
			filters::Error::InvalidFilterConfiguration(format!(
				"Invalid IP address in X-Forwarded-For header: {}",
				ips[index]
			))
		})
	}
}

#[cfg(test)]
#[path = "ipallowlist_tests.rs"]
mod tests;
