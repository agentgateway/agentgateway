use std::net::IpAddr;

use ipnet::IpNet;
use macro_rules_attribute::apply;
use once_cell::sync::Lazy;
use tracing::debug;

use crate::cel::SourceContext;
use crate::http::Request;
use crate::proxy::ProxyError;
use crate::*;

#[cfg(test)]
#[path = "ipallowlist_tests.rs"]
mod tests;

const DEFAULT_MAX_XFF_IPS: usize = 30;

static PRIVATE_RANGES: Lazy<Vec<IpNet>> = Lazy::new(|| {
	[
		"10.0.0.0/8",
		"172.16.0.0/12",
		"192.168.0.0/16",
		"127.0.0.0/8",
		"::1/128",
		"fc00::/7",
		"fe80::/10",
	]
	.iter()
	.map(|s| s.parse().unwrap())
	.collect()
});

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("ip address {0} is not allowed")]
	Denied(IpAddr),
	#[error("missing source context")]
	MissingSourceContext,
	#[error("X-Forwarded-For chain length {actual} exceeds maximum {max}")]
	XffTooLong { actual: usize, max: usize },
}

/// IP access control policy supporting allow and deny lists with CIDR ranges.
///
/// Evaluation order:
/// 1. If the IP matches any deny entry, reject (403).
/// 2. If an allow list is configured and the IP does not match, reject (403).
/// 3. Otherwise, allow.
///
/// When `xff_num_trusted_hops` is set, the client IP is extracted from the
/// X-Forwarded-For header (counting from the right) instead of the direct
/// connection IP.
#[apply(schema!)]
#[derive(PartialEq, Eq)]
pub struct IpAccessControl {
	/// CIDR ranges that are allowed. When non-empty, only IPs matching at least
	/// one entry are permitted (after deny check).
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
	pub allow: Vec<IpNet>,

	/// CIDR ranges that are denied. Deny rules are evaluated before allow rules.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
	pub deny: Vec<IpNet>,

	/// Number of trusted proxy hops. When set, the client IP is taken from the
	/// X-Forwarded-For header, counting backwards from the rightmost entry.
	/// For example, 1 means the last entry is the client IP (one trusted proxy).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub xff_num_trusted_hops: Option<usize>,

	/// When true, private/loopback IPs (RFC 1918, RFC 4193, link-local) bypass
	/// both allow and deny checks. Defaults to false.
	#[serde(default, skip_serializing_if = "is_false")]
	pub skip_private_ips: bool,

	/// When true, every IP in the X-Forwarded-For chain (plus the connection IP)
	/// is checked against allow/deny rules. A deny match on any hop rejects the
	/// request, and every non-private hop must satisfy the allow list.
	/// When false, only the resolved client IP is checked.
	#[serde(default, skip_serializing_if = "is_false")]
	pub enforce_full_chain: bool,

	/// Maximum number of IPs allowed in the X-Forwarded-For chain. Requests
	/// exceeding this limit are rejected with 403. Guards against forwarding
	/// loops and header abuse. Defaults to 30 when unset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_xff_length: Option<usize>,
}

fn is_false(v: &bool) -> bool {
	!v
}

fn is_private(ip: &IpAddr) -> bool {
	PRIVATE_RANGES.iter().any(|net| net.contains(ip))
}

impl IpAccessControl {
	/// Merge multiple `IpAccessControl` policies into one. This is used when
	/// policies are attached at different levels (e.g., gateway-wide by the
	/// operator and listener-level by a client). The semantics are:
	///
	/// - **allow**: union -- an IP matching any source's allow list is permitted.
	/// - **deny**: union -- a deny entry from any source blocks.
	/// - **Behavioral flags** (`xff_num_trusted_hops`, `skip_private_ips`,
	///   `enforce_full_chain`, `max_xff_length`): the first (most specific)
	///   non-default value wins.
	///
	/// Policies earlier in the iterator are considered more specific (listener
	/// before gateway).
	pub fn merge(policies: impl IntoIterator<Item = IpAccessControl>) -> Option<Self> {
		let mut allow = Vec::new();
		let mut deny = Vec::new();
		let mut xff_num_trusted_hops: Option<usize> = None;
		let mut skip_private_ips: Option<bool> = None;
		let mut enforce_full_chain: Option<bool> = None;
		let mut max_xff_length: Option<usize> = None;
		let mut count = 0;

		for p in policies {
			count += 1;
			allow.extend(p.allow);
			deny.extend(p.deny);
			if xff_num_trusted_hops.is_none() && p.xff_num_trusted_hops.is_some() {
				xff_num_trusted_hops = p.xff_num_trusted_hops;
			}
			if skip_private_ips.is_none() && p.skip_private_ips {
				skip_private_ips = Some(true);
			}
			if enforce_full_chain.is_none() && p.enforce_full_chain {
				enforce_full_chain = Some(true);
			}
			if max_xff_length.is_none() && p.max_xff_length.is_some() {
				max_xff_length = p.max_xff_length;
			}
		}

		if count == 0 {
			return None;
		}

		allow.sort_by_key(|a| a.to_string());
		allow.dedup();
		deny.sort_by_key(|a| a.to_string());
		deny.dedup();

		Some(IpAccessControl {
			allow,
			deny,
			xff_num_trusted_hops,
			skip_private_ips: skip_private_ips.unwrap_or(false),
			enforce_full_chain: enforce_full_chain.unwrap_or(false),
			max_xff_length,
		})
	}

	pub fn apply(&self, req: &Request) -> Result<(), ProxyError> {
		let xff_parts = self.parse_xff(req);

		self
			.validate_xff_length(&xff_parts)
			.map_err(ProxyError::IpAccessDenied)?;

		if self.enforce_full_chain {
			self.apply_full_chain(req, &xff_parts)
		} else {
			let ip = self.resolve_client_ip(req, &xff_parts)?;
			self.check(ip).map_err(ProxyError::IpAccessDenied)
		}
	}

	fn parse_xff<'a>(&self, req: &'a Request) -> Vec<&'a str> {
		req
			.headers()
			.get("x-forwarded-for")
			.and_then(|v| v.to_str().ok())
			.map(|s| s.split(',').map(|part| part.trim()).collect())
			.unwrap_or_default()
	}

	fn validate_xff_length(&self, xff_parts: &[&str]) -> Result<(), Error> {
		let max = self.max_xff_length.unwrap_or(DEFAULT_MAX_XFF_IPS);
		if xff_parts.len() > max {
			return Err(Error::XffTooLong {
				actual: xff_parts.len(),
				max,
			});
		}
		Ok(())
	}

	fn resolve_client_ip(&self, req: &Request, xff_parts: &[&str]) -> Result<IpAddr, ProxyError> {
		if let Some(hops) = self.xff_num_trusted_hops
			&& !xff_parts.is_empty()
		{
			let idx = xff_parts.len().saturating_sub(hops);
			if let Some(ip_str) = xff_parts.get(idx) {
				if let Ok(ip) = ip_str.parse::<IpAddr>() {
					return Ok(ip);
				}
				debug!(
					hop_index = idx,
					entry = ip_str,
					"failed to parse XFF IP entry"
				);
			}
		}

		req
			.extensions()
			.get::<SourceContext>()
			.map(|src| src.address)
			.ok_or(ProxyError::IpAccessDenied(Error::MissingSourceContext))
	}

	fn apply_full_chain(&self, req: &Request, xff_parts: &[&str]) -> Result<(), ProxyError> {
		let conn_ip = req
			.extensions()
			.get::<SourceContext>()
			.map(|src| src.address);

		let xff_ips = xff_parts.iter().filter_map(|s| s.parse::<IpAddr>().ok());

		let all_ips: Vec<IpAddr> = xff_ips.chain(conn_ip).collect();

		for ip in all_ips {
			self.check(ip).map_err(ProxyError::IpAccessDenied)?;
		}

		Ok(())
	}

	fn check(&self, ip: IpAddr) -> Result<(), Error> {
		if self.skip_private_ips && is_private(&ip) {
			return Ok(());
		}

		if self.deny.iter().any(|net| net.contains(&ip)) {
			return Err(Error::Denied(ip));
		}

		if !self.allow.is_empty() && !self.allow.iter().any(|net| net.contains(&ip)) {
			return Err(Error::Denied(ip));
		}

		Ok(())
	}
}
