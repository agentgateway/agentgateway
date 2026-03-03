use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::IpNet;

use super::*;
use crate::cel::SourceContext;
use crate::types::agent::TrafficPolicy;

fn make_request_with_ip(ip: IpAddr) -> Request {
	let mut req = ::http::Request::builder()
		.uri("http://example.com/")
		.body(crate::http::Body::empty())
		.unwrap();
	req.extensions_mut().insert(SourceContext {
		address: ip,
		port: 12345,
		tls: None,
	});
	req
}

fn make_request_with_ip_and_xff(ip: IpAddr, xff: &str) -> Request {
	let mut req = make_request_with_ip(ip);
	req.headers_mut().insert(
		"x-forwarded-for",
		::http::HeaderValue::from_str(xff).unwrap(),
	);
	req
}

fn cidr(s: &str) -> IpNet {
	s.parse().unwrap()
}

fn default_policy() -> IpAccessControl {
	IpAccessControl {
		allow: vec![],
		deny: vec![],
		xff_num_trusted_hops: None,
		skip_private_ips: false,
		enforce_full_chain: false,
		max_xff_length: None,
	}
}

// ── Basic allow / deny ──

#[test]
fn allow_all_when_empty() {
	let policy = default_policy();
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn allow_list_permits_matching_ip() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn allow_list_rejects_non_matching_ip() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn deny_list_rejects_matching_ip() {
	let policy = IpAccessControl {
		deny: vec![cidr("192.168.0.0/16")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn deny_list_allows_non_matching_ip() {
	let policy = IpAccessControl {
		deny: vec![cidr("192.168.0.0/16")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn deny_takes_precedence_over_allow() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		deny: vec![cidr("10.0.1.0/24")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)));
	assert!(policy.apply(&req).is_err());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 5)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn ipv6_support() {
	let policy = IpAccessControl {
		allow: vec![cidr("fd00::/8")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V6("fd00::1".parse().unwrap()));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V6("2001:db8::1".parse().unwrap()));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn single_ip_cidr() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.1/32")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
	assert!(policy.apply(&req).is_err());
}

// ── XFF hops ──

#[test]
fn xff_single_trusted_hop() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(1),
		..default_policy()
	};
	// XFF: "10.0.0.5, 172.16.0.1"
	// With 1 trusted hop: idx = 2 - 1 = 1, parts[1] = "172.16.0.1" not in 10/8
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.5, 172.16.0.1",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn xff_two_trusted_hops() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(2),
		..default_policy()
	};
	// idx = 3 - 2 = 1, parts[1] = "172.16.0.1" not in 10/8
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.5, 172.16.0.1, 172.16.0.2",
	);
	assert!(policy.apply(&req).is_err());

	// idx = 3 - 2 = 1, parts[1] = "10.0.0.6" IS in 10/8
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.5, 10.0.0.6, 172.16.0.2",
	);
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_falls_back_to_connection_ip_when_no_header() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(1),
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_disabled_ignores_header() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let req = make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), "192.168.1.1");
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn multiple_allow_ranges() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8"), cidr("172.16.0.0/12")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(172, 20, 0, 1)));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn check_method_directly() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		deny: vec![cidr("10.0.1.0/24")],
		..default_policy()
	};

	assert!(policy.check(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 1))).is_ok());
	assert!(
		policy
			.check(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1)))
			.is_err()
	);
	assert!(
		policy
			.check(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
			.is_err()
	);
}

// ── Private IP bypass ──

#[test]
fn skip_private_ips_bypasses_allow_list() {
	let policy = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		skip_private_ips: true,
		..default_policy()
	};
	// 10.0.0.1 is private -> should bypass the allowlist and pass
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());

	// 203.0.113.5 is in the allowlist -> passes
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
	assert!(policy.apply(&req).is_ok());

	// 8.8.8.8 is public and not in the allowlist -> rejected
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn skip_private_ips_bypasses_deny_list() {
	let policy = IpAccessControl {
		deny: vec![cidr("10.0.0.0/8")],
		skip_private_ips: true,
		..default_policy()
	};
	// 10.0.0.1 matches deny but is private -> bypassed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());

	// 192.168.1.1 is private -> bypassed even though not in any list
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_still_denies_public_ips() {
	let policy = IpAccessControl {
		deny: vec![cidr("8.8.8.0/24")],
		skip_private_ips: true,
		..default_policy()
	};
	// 8.8.8.8 is public and in deny -> rejected
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(policy.apply(&req).is_err());

	// 1.1.1.1 is public and not in deny -> allowed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_loopback_v4() {
	let policy = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		skip_private_ips: true,
		..default_policy()
	};
	// 127.0.0.1 is loopback -> bypassed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_loopback_v6() {
	let policy = IpAccessControl {
		allow: vec![cidr("2001:db8::/32")],
		skip_private_ips: true,
		..default_policy()
	};
	// ::1 is IPv6 loopback -> bypassed
	let req = make_request_with_ip(IpAddr::V6(Ipv6Addr::LOCALHOST));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_ula_v6() {
	let policy = IpAccessControl {
		allow: vec![cidr("2001:db8::/32")],
		skip_private_ips: true,
		..default_policy()
	};
	// fc00::1 is ULA (private IPv6) -> bypassed
	let req = make_request_with_ip(IpAddr::V6("fc00::1".parse().unwrap()));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_link_local_v6() {
	let policy = IpAccessControl {
		allow: vec![cidr("2001:db8::/32")],
		skip_private_ips: true,
		..default_policy()
	};
	// fe80::1 is link-local -> bypassed
	let req = make_request_with_ip(IpAddr::V6("fe80::1".parse().unwrap()));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn skip_private_ips_disabled_does_not_bypass() {
	let policy = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		skip_private_ips: false,
		..default_policy()
	};
	// 10.0.0.1 is private but bypass is disabled -> rejected by allowlist
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_err());
}

// ── XFF length limit ──

#[test]
fn xff_length_limit_rejects_oversized_chain() {
	let policy = IpAccessControl {
		max_xff_length: Some(3),
		..default_policy()
	};
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
		"10.0.0.1, 10.0.0.2, 10.0.0.3, 10.0.0.4",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn xff_length_limit_allows_within_limit() {
	let policy = IpAccessControl {
		max_xff_length: Some(3),
		..default_policy()
	};
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
		"10.0.0.1, 10.0.0.2, 10.0.0.3",
	);
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_length_limit_allows_exact_boundary() {
	let policy = IpAccessControl {
		max_xff_length: Some(2),
		..default_policy()
	};
	// Exactly 2 entries -> allowed
	let req =
		make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), "10.0.0.1, 10.0.0.2");
	assert!(policy.apply(&req).is_ok());

	// 3 entries -> rejected
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)),
		"10.0.0.1, 10.0.0.2, 10.0.0.3",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn xff_length_default_limit_allows_30() {
	let policy = default_policy();
	let ips: Vec<String> = (1..=30).map(|i| format!("10.0.0.{i}")).collect();
	let xff = ips.join(", ");
	let req = make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), &xff);
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_length_default_limit_rejects_31() {
	let policy = default_policy();
	let ips: Vec<String> = (1..=31).map(|i| format!("10.0.{}", i % 256)).collect();
	let xff = ips.join(", ");
	let req = make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), &xff);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn xff_length_no_xff_header_always_passes() {
	let policy = IpAccessControl {
		max_xff_length: Some(1),
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)));
	assert!(policy.apply(&req).is_ok());
}

// ── Enforce full chain ──

#[test]
fn full_chain_rejects_if_any_hop_denied() {
	let policy = IpAccessControl {
		deny: vec![cidr("172.16.0.0/12")],
		enforce_full_chain: true,
		..default_policy()
	};
	// Connection IP is fine, but one XFF hop matches deny
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
		"10.0.0.2, 172.16.0.5, 10.0.0.3",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn full_chain_allows_when_all_hops_clean() {
	let policy = IpAccessControl {
		deny: vec![cidr("172.16.0.0/12")],
		enforce_full_chain: true,
		..default_policy()
	};
	let req =
		make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), "10.0.0.2, 10.0.0.3");
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn full_chain_rejects_connection_ip_too() {
	let policy = IpAccessControl {
		deny: vec![cidr("192.168.0.0/16")],
		enforce_full_chain: true,
		..default_policy()
	};
	// All XFF hops are fine but the connection IP itself is denied
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.1, 10.0.0.2",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn full_chain_allowlist_every_hop_must_pass() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		enforce_full_chain: true,
		..default_policy()
	};
	// One XFF hop outside the allowlist
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
		"10.0.0.2, 8.8.8.8, 10.0.0.3",
	);
	assert!(policy.apply(&req).is_err());

	// All hops within the allowlist
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
		"10.0.0.2, 10.0.0.3, 10.0.0.4",
	);
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn full_chain_with_private_bypass() {
	let policy = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		enforce_full_chain: true,
		skip_private_ips: true,
		..default_policy()
	};
	// XFF contains private 10.x and allowed 203.0.113.x, connection is private 192.168.x
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.1, 203.0.113.5",
	);
	assert!(policy.apply(&req).is_ok());

	// Public IP not in allow list -> rejected
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.1, 8.8.8.8",
	);
	assert!(policy.apply(&req).is_err());
}

#[test]
fn full_chain_skips_unparseable_xff_entries() {
	let policy = IpAccessControl {
		deny: vec![cidr("172.16.0.0/12")],
		enforce_full_chain: true,
		..default_policy()
	};
	// "garbage" entry is silently skipped; remaining IPs are clean
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
		"10.0.0.2, garbage, 10.0.0.3",
	);
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn full_chain_no_xff_header_checks_connection_only() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		enforce_full_chain: true,
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(policy.apply(&req).is_err());
}

// ── Combined features ──

#[test]
fn xff_limit_checked_before_full_chain() {
	let policy = IpAccessControl {
		max_xff_length: Some(2),
		enforce_full_chain: true,
		..default_policy()
	};
	// 3 XFF entries exceed limit=2 -> rejected even though IPs are all fine
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
		"10.0.0.2, 10.0.0.3, 10.0.0.4",
	);
	assert!(policy.apply(&req).is_err());
}

// ── Edge cases ──

#[test]
fn missing_source_context_returns_error() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let req = ::http::Request::builder()
		.uri("http://example.com/")
		.body(crate::http::Body::empty())
		.unwrap();
	// No SourceContext inserted -> should error
	assert!(policy.apply(&req).is_err());
}

#[test]
fn missing_source_context_full_chain_still_checks_xff() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		enforce_full_chain: true,
		..default_policy()
	};
	let mut req = ::http::Request::builder()
		.uri("http://example.com/")
		.body(crate::http::Body::empty())
		.unwrap();
	// No SourceContext, but XFF has allowed IPs
	req.headers_mut().insert(
		"x-forwarded-for",
		::http::HeaderValue::from_static("10.0.0.1, 10.0.0.2"),
	);
	// No conn_ip in chain -> only XFF IPs checked, all allowed -> ok
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_hops_exceeds_chain_length_uses_first_entry() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(10),
		..default_policy()
	};
	// Only 2 entries, hops=10 -> saturating_sub gives idx=0 -> uses first entry
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"10.0.0.5, 172.16.0.1",
	);
	// idx=0, parts[0]="10.0.0.5" which IS in 10/8 -> allowed
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_unparseable_entry_falls_back_to_connection_ip() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(1),
		..default_policy()
	};
	// XFF has one entry but it's garbage -> falls back to connection IP
	let req = make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), "not-an-ip");
	// Connection IP is 10.0.0.1 which is in allow -> ok
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn ipv6_deny() {
	let policy = IpAccessControl {
		deny: vec![cidr("2001:db8::/32")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V6("2001:db8::1".parse().unwrap()));
	assert!(policy.apply(&req).is_err());

	let req = make_request_with_ip(IpAddr::V6("fd00::1".parse().unwrap()));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn ipv6_single_host_cidr() {
	let policy = IpAccessControl {
		allow: vec![cidr("2001:db8::1/128")],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V6("2001:db8::1".parse().unwrap()));
	assert!(policy.apply(&req).is_ok());

	let req = make_request_with_ip(IpAddr::V6("2001:db8::2".parse().unwrap()));
	assert!(policy.apply(&req).is_err());
}

#[test]
fn multiple_deny_ranges() {
	let policy = IpAccessControl {
		deny: vec![
			cidr("192.168.0.0/16"),
			cidr("172.16.0.0/12"),
			cidr("10.0.1.0/24"),
		],
		..default_policy()
	};
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(policy.apply(&req).is_err());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(172, 20, 0, 1)));
	assert!(policy.apply(&req).is_err());

	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)));
	assert!(policy.apply(&req).is_err());

	// Not in any deny range -> allowed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_whitespace_handling() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		xff_num_trusted_hops: Some(1),
		..default_policy()
	};
	// Extra whitespace around the IP entries
	let req = make_request_with_ip_and_xff(
		IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
		"  10.0.0.5  ,  10.0.0.6  ",
	);
	// idx = 2 - 1 = 1, parts[1] = "10.0.0.6" (trimmed) which IS in 10/8
	assert!(policy.apply(&req).is_ok());
}

#[test]
fn xff_limit_rejects_before_single_ip_check() {
	let policy = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		max_xff_length: Some(1),
		xff_num_trusted_hops: Some(1),
		..default_policy()
	};
	// 2 XFF entries > limit of 1 -> rejected before any IP check
	let req =
		make_request_with_ip_and_xff(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), "10.0.0.2, 10.0.0.3");
	assert!(policy.apply(&req).is_err());
}

// ── Merge ──

#[test]
fn merge_empty_returns_none() {
	assert!(IpAccessControl::merge(vec![]).is_none());
}

#[test]
fn merge_single_policy_is_identity() {
	let p = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		deny: vec![cidr("192.168.0.0/16")],
		xff_num_trusted_hops: Some(2),
		skip_private_ips: true,
		enforce_full_chain: true,
		max_xff_length: Some(10),
	};
	let merged = IpAccessControl::merge(vec![p.clone()]).unwrap();
	assert_eq!(merged.allow, vec![cidr("10.0.0.0/8")]);
	assert_eq!(merged.deny, vec![cidr("192.168.0.0/16")]);
	assert_eq!(merged.xff_num_trusted_hops, Some(2));
	assert!(merged.skip_private_ips);
	assert!(merged.enforce_full_chain);
	assert_eq!(merged.max_xff_length, Some(10));
}

#[test]
fn merge_unions_allow_lists() {
	let operator = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let client = IpAccessControl {
		allow: vec![cidr("172.16.0.0/12")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![operator, client]).unwrap();
	assert_eq!(
		merged.allow,
		vec![cidr("10.0.0.0/8"), cidr("172.16.0.0/12")]
	);
}

#[test]
fn merge_unions_deny_lists() {
	let operator = IpAccessControl {
		deny: vec![cidr("192.168.0.0/16")],
		..default_policy()
	};
	let client = IpAccessControl {
		deny: vec![cidr("10.0.1.0/24")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![operator, client]).unwrap();
	assert_eq!(merged.deny.len(), 2);
	assert!(merged.deny.contains(&cidr("192.168.0.0/16")));
	assert!(merged.deny.contains(&cidr("10.0.1.0/24")));
}

#[test]
fn merge_unions_both_allow_and_deny() {
	let operator = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		deny: vec![cidr("10.0.1.0/24")],
		..default_policy()
	};
	let client = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		deny: vec![cidr("10.0.2.0/24")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![operator, client]).unwrap();

	// IP in operator's allow range but not in operator's deny -> allowed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(merged.apply(&req).is_ok());

	// IP in client's allow range -> allowed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
	assert!(merged.apply(&req).is_ok());

	// IP in operator's deny range -> denied even though in operator's allow range
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)));
	assert!(merged.apply(&req).is_err());

	// IP in client's deny range -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 5)));
	assert!(merged.apply(&req).is_err());

	// IP not in any allow range -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(merged.apply(&req).is_err());
}

#[test]
fn merge_first_policy_flags_win() {
	// Listener-level (more specific, comes first) sets flags
	let listener = IpAccessControl {
		xff_num_trusted_hops: Some(3),
		skip_private_ips: true,
		enforce_full_chain: true,
		max_xff_length: Some(5),
		..default_policy()
	};
	// Gateway-level (less specific, comes second) also sets flags
	let gateway = IpAccessControl {
		xff_num_trusted_hops: Some(1),
		skip_private_ips: false,
		enforce_full_chain: false,
		max_xff_length: Some(50),
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![listener, gateway]).unwrap();
	assert_eq!(
		merged.xff_num_trusted_hops,
		Some(3),
		"listener's value wins"
	);
	assert!(merged.skip_private_ips, "listener's true wins");
	assert!(merged.enforce_full_chain, "listener's true wins");
	assert_eq!(merged.max_xff_length, Some(5), "listener's value wins");
}

#[test]
fn merge_falls_back_to_second_policy_flags_when_first_is_default() {
	let listener = default_policy(); // all flags are default
	let gateway = IpAccessControl {
		xff_num_trusted_hops: Some(2),
		skip_private_ips: true,
		enforce_full_chain: true,
		max_xff_length: Some(20),
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![listener, gateway]).unwrap();
	assert_eq!(merged.xff_num_trusted_hops, Some(2), "gateway's value used");
	assert!(merged.skip_private_ips, "gateway's true used");
	assert!(merged.enforce_full_chain, "gateway's true used");
	assert_eq!(merged.max_xff_length, Some(20), "gateway's value used");
}

#[test]
fn merge_deduplicates_identical_ranges() {
	let a = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8"), cidr("172.16.0.0/12")],
		deny: vec![cidr("192.168.0.0/16")],
		..default_policy()
	};
	let b = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		deny: vec![cidr("192.168.0.0/16"), cidr("10.0.1.0/24")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![a, b]).unwrap();
	assert_eq!(merged.allow.len(), 2, "duplicates removed from allow");
	assert!(merged.allow.contains(&cidr("10.0.0.0/8")));
	assert!(merged.allow.contains(&cidr("172.16.0.0/12")));
	assert_eq!(merged.deny.len(), 2, "duplicates removed from deny");
	assert!(merged.deny.contains(&cidr("192.168.0.0/16")));
	assert!(merged.deny.contains(&cidr("10.0.1.0/24")));
}

#[test]
fn merge_operator_deny_client_allow_interaction() {
	// Operator denies a subnet; client allows a broader range
	let operator = IpAccessControl {
		deny: vec![cidr("10.0.1.0/24")],
		..default_policy()
	};
	let client = IpAccessControl {
		allow: vec![cidr("10.0.0.0/8")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![operator, client]).unwrap();

	// 10.0.2.1 is in client's allow and not in operator's deny -> allowed
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 1)));
	assert!(merged.apply(&req).is_ok());

	// 10.0.1.5 is in client's allow BUT in operator's deny -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)));
	assert!(merged.apply(&req).is_err());

	// 8.8.8.8 is not in client's allow -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(merged.apply(&req).is_err());
}

#[test]
fn merge_three_sources() {
	let infra = IpAccessControl {
		deny: vec![cidr("198.51.100.0/24")],
		skip_private_ips: true,
		..default_policy()
	};
	let operator = IpAccessControl {
		allow: vec![cidr("203.0.113.0/24")],
		deny: vec![cidr("203.0.113.128/25")],
		..default_policy()
	};
	let client = IpAccessControl {
		allow: vec![cidr("100.64.0.0/10")],
		..default_policy()
	};
	let merged = IpAccessControl::merge(vec![infra, operator, client]).unwrap();

	assert!(merged.skip_private_ips, "infra's true wins");

	// 203.0.113.5 in operator's allow, not in operator's deny -> ok
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5)));
	assert!(merged.apply(&req).is_ok());

	// 100.64.0.1 in client's allow -> ok
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)));
	assert!(merged.apply(&req).is_ok());

	// 203.0.113.200 in operator's deny (128/25 covers .128-.255) -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 200)));
	assert!(merged.apply(&req).is_err());

	// 198.51.100.1 in infra's deny -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)));
	assert!(merged.apply(&req).is_err());

	// 192.168.1.1 is private and skip_private_ips is on -> ok (bypassed)
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(merged.apply(&req).is_ok());

	// 8.8.8.8 is public, not in any allow list -> denied
	let req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
	assert!(merged.apply(&req).is_err());
}

// MARK: xDS proto conversion tests

fn make_proto_ip_access_control(
	allow: Vec<&str>,
	deny: Vec<&str>,
	xff_hops: Option<u32>,
	skip_private: bool,
	full_chain: bool,
	max_xff: Option<u32>,
) -> crate::types::proto::agent::TrafficPolicySpec {
	use crate::types::proto::agent::traffic_policy_spec;
	crate::types::proto::agent::TrafficPolicySpec {
		phase: traffic_policy_spec::PolicyPhase::Gateway as i32,
		kind: Some(traffic_policy_spec::Kind::IpAccessControl(
			traffic_policy_spec::IpAccessControl {
				allow: allow.into_iter().map(String::from).collect(),
				deny: deny.into_iter().map(String::from).collect(),
				xff_num_trusted_hops: xff_hops,
				skip_private_ips: skip_private,
				enforce_full_chain: full_chain,
				max_xff_length: max_xff,
			},
		)),
	}
}

#[test]
fn xds_roundtrip_basic_allow_deny() {
	let proto = make_proto_ip_access_control(
		vec!["10.0.0.0/8", "172.16.0.0/12"],
		vec!["10.0.1.0/24"],
		None,
		false,
		false,
		None,
	);
	let tp = TrafficPolicy::try_from(&proto).expect("conversion should succeed");
	match tp {
		TrafficPolicy::IpAccessControl(iac) => {
			assert_eq!(iac.allow.len(), 2);
			assert_eq!(iac.deny.len(), 1);
			assert_eq!(iac.allow[0], "10.0.0.0/8".parse::<IpNet>().unwrap());
			assert_eq!(iac.allow[1], "172.16.0.0/12".parse::<IpNet>().unwrap());
			assert_eq!(iac.deny[0], "10.0.1.0/24".parse::<IpNet>().unwrap());
			assert!(iac.xff_num_trusted_hops.is_none());
			assert!(!iac.skip_private_ips);
			assert!(!iac.enforce_full_chain);
			assert!(iac.max_xff_length.is_none());
		},
		other => panic!("expected IpAccessControl, got {other:?}"),
	}
}

#[test]
fn xds_roundtrip_all_flags() {
	let proto = make_proto_ip_access_control(
		vec!["192.168.0.0/16"],
		vec![],
		Some(2),
		true,
		true,
		Some(50),
	);
	let tp = TrafficPolicy::try_from(&proto).expect("conversion should succeed");
	match tp {
		TrafficPolicy::IpAccessControl(iac) => {
			assert_eq!(iac.allow.len(), 1);
			assert!(iac.deny.is_empty());
			assert_eq!(iac.xff_num_trusted_hops, Some(2));
			assert!(iac.skip_private_ips);
			assert!(iac.enforce_full_chain);
			assert_eq!(iac.max_xff_length, Some(50));
		},
		other => panic!("expected IpAccessControl, got {other:?}"),
	}
}

#[test]
fn xds_roundtrip_empty_lists() {
	let proto = make_proto_ip_access_control(vec![], vec![], None, false, false, None);
	let tp = TrafficPolicy::try_from(&proto).expect("conversion should succeed");
	match tp {
		TrafficPolicy::IpAccessControl(iac) => {
			assert!(iac.allow.is_empty());
			assert!(iac.deny.is_empty());
		},
		other => panic!("expected IpAccessControl, got {other:?}"),
	}
}

#[test]
fn xds_roundtrip_invalid_cidr_rejected() {
	let proto = make_proto_ip_access_control(vec!["not-a-cidr"], vec![], None, false, false, None);
	let result = TrafficPolicy::try_from(&proto);
	assert!(
		result.is_err(),
		"invalid CIDR should cause conversion error"
	);
}

#[test]
fn xds_roundtrip_ipv6_cidrs() {
	let proto = make_proto_ip_access_control(
		vec!["fd00::/8", "2001:db8::/32"],
		vec!["::1/128"],
		Some(1),
		false,
		false,
		None,
	);
	let tp = TrafficPolicy::try_from(&proto).expect("conversion should succeed");
	match tp {
		TrafficPolicy::IpAccessControl(iac) => {
			assert_eq!(iac.allow.len(), 2);
			assert_eq!(iac.deny.len(), 1);
			assert_eq!(iac.xff_num_trusted_hops, Some(1));
		},
		other => panic!("expected IpAccessControl, got {other:?}"),
	}
}

#[test]
fn xds_roundtrip_functional_apply() {
	let proto = make_proto_ip_access_control(
		vec!["10.0.0.0/8"],
		vec!["10.0.1.0/24"],
		None,
		false,
		false,
		None,
	);
	let tp = TrafficPolicy::try_from(&proto).expect("conversion should succeed");
	let iac = match tp {
		TrafficPolicy::IpAccessControl(iac) => iac,
		other => panic!("expected IpAccessControl, got {other:?}"),
	};

	let ok_req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
	assert!(iac.apply(&ok_req).is_ok(), "10.0.0.1 should be allowed");

	let deny_req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 5)));
	assert!(iac.apply(&deny_req).is_err(), "10.0.1.5 should be denied");

	let no_match_req = make_request_with_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
	assert!(no_match_req.extensions().get::<SourceContext>().is_some());
	assert!(
		iac.apply(&no_match_req).is_err(),
		"192.168.1.1 not in allow list"
	);
}
