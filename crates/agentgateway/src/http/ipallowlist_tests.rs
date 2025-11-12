use std::net::{IpAddr, SocketAddr};

use http_body_util::BodyExt;

use crate::http::StatusCode;
use crate::http::ipallowlist::{IpAllowlist, IpAllowlistSerde, IpSource};
use crate::http::tests_common::*;

/// Helper to create a request with a socket address in extensions
fn request_with_ip(ip: &str) -> crate::http::Request {
	let mut req = request_for_uri("http://test.com/api");
	let ip_addr: IpAddr = ip.parse().unwrap();
	let socket_addr = SocketAddr::new(ip_addr, 12345);
	req.extensions_mut().insert(socket_addr);
	req
}

/// Helper to create a request with X-Forwarded-For header
fn request_with_xff(xff: &str) -> crate::http::Request {
	request(
		"http://test.com/api",
		http::Method::GET,
		&[("X-Forwarded-For", xff)],
	)
}

// ==================== Basic IP Matching Tests ====================

#[test]
fn test_wildcard_allows_all() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["*".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Test IPv4
	let mut req = request_with_ip("192.168.1.100");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"wildcard should allow IPv4"
	);

	// Test IPv6
	let mut req = request_with_ip("2001:db8::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"wildcard should allow IPv6"
	);
}

#[test]
fn test_empty_list_denies_all() {
	let config = IpAllowlistSerde {
		allowed_ips: vec![],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	let mut req = request_with_ip("192.168.1.100");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"empty list should deny all"
	);

	let response = result.direct_response.unwrap();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn test_single_ipv4_allowed() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.100".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Matching IP should be allowed
	let mut req = request_with_ip("192.168.1.100");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"matching IP should be allowed"
	);

	// Different IP should be denied
	let mut req = request_with_ip("192.168.1.101");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"non-matching IP should be denied"
	);

	let response = result.direct_response.unwrap();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[test]
fn test_single_ipv6_allowed() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["2001:db8::1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Matching IP should be allowed
	let mut req = request_with_ip("2001:db8::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"matching IPv6 should be allowed"
	);

	// Different IP should be denied
	let mut req = request_with_ip("2001:db8::2");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"non-matching IPv6 should be denied"
	);
}

#[test]
fn test_cidr_ipv4_range() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.0/24".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// IPs in range should be allowed
	let test_cases = vec![
		("192.168.1.1", true),
		("192.168.1.100", true),
		("192.168.1.254", true),
		("192.168.2.1", false),
		("192.168.0.255", false),
		("10.0.0.1", false),
	];

	for (ip, should_allow) in test_cases {
		let mut req = request_with_ip(ip);
		let result = allowlist.apply(&mut req).unwrap();
		if should_allow {
			assert!(result.direct_response.is_none(), "{} should be allowed", ip);
		} else {
			assert!(result.direct_response.is_some(), "{} should be denied", ip);
		}
	}
}

#[test]
fn test_cidr_ipv6_range() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["2001:db8::/32".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// IPs in range should be allowed
	let mut req = request_with_ip("2001:db8::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"IP in range should be allowed"
	);

	let mut req = request_with_ip("2001:db8:1::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"IP in range should be allowed"
	);

	// IP out of range should be denied
	let mut req = request_with_ip("2001:db9::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"IP out of range should be denied"
	);
}

#[test]
fn test_multiple_ips_and_ranges() {
	let config = IpAllowlistSerde {
		allowed_ips: vec![
			"10.0.0.1".to_string(),
			"192.168.1.0/24".to_string(),
			"2001:db8::1".to_string(),
		],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Test single IP match
	let mut req = request_with_ip("10.0.0.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"single IP should be allowed"
	);

	// Test CIDR range match
	let mut req = request_with_ip("192.168.1.50");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"IP in CIDR range should be allowed"
	);

	// Test IPv6 match
	let mut req = request_with_ip("2001:db8::1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_none(), "IPv6 should be allowed");

	// Test non-matching IP
	let mut req = request_with_ip("172.16.0.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"non-matching IP should be denied"
	);
}

// ==================== Custom Status Code and Message Tests ====================

#[test]
fn test_custom_deny_status_code() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["10.0.0.1".to_string()],
		deny_status_code: 404,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	let mut req = request_with_ip("192.168.1.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_some());

	let response = result.direct_response.unwrap();
	assert_eq!(
		response.status(),
		StatusCode::NOT_FOUND,
		"should use custom status code 404"
	);
}

#[tokio::test]
async fn test_custom_deny_message() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["10.0.0.1".to_string()],
		deny_status_code: 403,
		deny_message: Some("Access denied: IP not authorized".to_string()),
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	let mut req = request_with_ip("192.168.1.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_some());

	let response = result.direct_response.unwrap();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);

	// Check body contains custom message
	let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
	let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
	assert_eq!(body_str, "Access denied: IP not authorized");
}

#[test]
fn test_custom_status_418_im_a_teapot() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["10.0.0.1".to_string()],
		deny_status_code: 418,
		deny_message: Some("I'm a teapot".to_string()),
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	let mut req = request_with_ip("192.168.1.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_some());

	let response = result.direct_response.unwrap();
	assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
}

// ==================== XFF Hop Selection Tests ====================

#[test]
fn test_xff_last_hop_default() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.5".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: 0, // Last hop (default)
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF: client, proxy1, proxy2, proxy3 -> should use proxy3 (last)
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3, 203.0.113.5");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"last hop (203.0.113.5) should be allowed"
	);

	// Different last hop should be denied
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3, 203.0.113.4");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_some(),
		"different last hop should be denied"
	);
}

#[test]
fn test_xff_second_to_last_hop() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.3".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -1, // Second to last
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF: client, proxy1, proxy2, proxy3 -> should use proxy2 (second to last)
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3, 203.0.113.4");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"second to last hop (203.0.113.3) should be allowed"
	);
}

#[test]
fn test_xff_third_to_last_hop() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.2".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -2, // Third to last
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF: client, proxy1, proxy2, proxy3 -> should use proxy1 (third to last)
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3, 203.0.113.4");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"third to last hop (203.0.113.2) should be allowed"
	);
}

#[test]
fn test_xff_fourth_to_last_hop() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -3, // Fourth to last (first in this case)
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF: client, proxy1, proxy2, proxy3 -> should use client (fourth to last = first)
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3, 203.0.113.4");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"fourth to last hop (203.0.113.1) should be allowed"
	);
}

#[test]
fn test_xff_first_hop_when_list_too_short() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -10, // Way beyond list length
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF: only 3 IPs, asking for 11th from last -> should use first
	let mut req = request_with_xff("203.0.113.1, 203.0.113.2, 203.0.113.3");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"should fallback to first hop when list too short"
	);
}

#[test]
fn test_xff_single_ip_in_header() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Single IP in XFF
	let mut req = request_with_xff("203.0.113.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_none(), "single IP should work");

	// Even with distance=-1, should still work (fallback to first)
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -1,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();
	let mut req = request_with_xff("203.0.113.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"should fallback to first when distance too far"
	);
}

#[test]
fn test_xff_with_whitespace() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.3".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF with extra whitespace
	let mut req = request_with_xff("  203.0.113.1  ,  203.0.113.2  ,  203.0.113.3  ");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"should handle whitespace correctly"
	);
}

#[test]
fn test_xff_with_cidr_ranges() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.0/24".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: -1, // Second to last
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// XFF with IP in CIDR range at second-to-last position
	let mut req = request_with_xff("10.0.0.1, 203.0.113.50, 192.168.1.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"CIDR range should match second-to-last IP"
	);
}

// ==================== IP Source Selection Tests ====================

#[test]
fn test_ip_source_remote_addr() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.100".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Request with socket addr should work
	let mut req = request_with_ip("192.168.1.100");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"RemoteAddr source should use socket address"
	);

	// Request with XFF but configured for RemoteAddr should fail (no socket addr)
	let mut req = request_with_xff("192.168.1.100");
	let result = allowlist.apply(&mut req);
	assert!(
		result.is_err(),
		"RemoteAddr source should fail without socket address"
	);
}

#[test]
fn test_ip_source_xff_ignores_socket_addr() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.5".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Create request with both socket addr and XFF - should use XFF
	let mut req = request(
		"http://test.com/api",
		http::Method::GET,
		&[("X-Forwarded-For", "203.0.113.5")],
	);
	let ip_addr: IpAddr = "192.168.1.100".parse().unwrap();
	let socket_addr = SocketAddr::new(ip_addr, 12345);
	req.extensions_mut().insert(socket_addr);

	// Should use XFF (203.0.113.5), not socket addr (192.168.1.100)
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"XForwardedFor source should use XFF header, not socket addr"
	);
}

#[test]
fn test_xff_source_prefers_xff_over_remote_addr() {
	// This is the UPDATED test - XFF is now the default and preferred
	let config = IpAllowlistSerde {
		allowed_ips: vec!["203.0.113.1".to_string()], // Allow the XFF IP
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor, // Explicitly XForwardedFor (also the default)
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Create request with BOTH socket addr AND XFF header
	let mut req = request(
		"http://test.com/api",
		http::Method::GET,
		&[("X-Forwarded-For", "203.0.113.1")],
	);
	// Socket addr is different IP
	let ip_addr: IpAddr = "192.168.1.100".parse().unwrap();
	let socket_addr = SocketAddr::new(ip_addr, 12345);
	req.extensions_mut().insert(socket_addr);

	// Should use XFF (203.0.113.1) and allow, ignoring socket addr (192.168.1.100)
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"should use XFF and allow when ip_source=XForwardedFor"
	);

	// Now test with RemoteAddr - should use socket address
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.100".to_string()], // Allow the socket IP
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	let mut req = request(
		"http://test.com/api",
		http::Method::GET,
		&[("X-Forwarded-For", "203.0.113.1")],
	);
	let ip_addr: IpAddr = "192.168.1.100".parse().unwrap();
	let socket_addr = SocketAddr::new(ip_addr, 12345);
	req.extensions_mut().insert(socket_addr);

	// Should use socket addr (192.168.1.100) and allow, ignoring XFF
	let result = allowlist.apply(&mut req).unwrap();
	assert!(
		result.direct_response.is_none(),
		"should use RemoteAddr when ip_source=RemoteAddr"
	);
}

#[test]
fn test_default_ip_source_is_xff() {
	let default_source = IpSource::default();
	assert_eq!(
		default_source,
		IpSource::XForwardedFor,
		"default IP source should be XForwardedFor"
	);
}

// ==================== Error Handling Tests ====================

#[test]
fn test_request_without_ip_fails_remote_addr() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.100".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Request with no socket addr and RemoteAddr source
	let mut req = request_for_uri("http://test.com/api");
	let result = allowlist.apply(&mut req);
	assert!(
		result.is_err(),
		"request without socket addr should fail for RemoteAddr source"
	);
}

#[test]
fn test_request_without_xff_header_fails() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.100".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::XForwardedFor,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Request with no XFF header and XForwardedFor source
	let mut req = request_for_uri("http://test.com/api");
	let result = allowlist.apply(&mut req);
	assert!(
		result.is_err(),
		"request without XFF header should fail for XForwardedFor source"
	);
}

#[test]
fn test_invalid_ip_in_config_fails() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["not-an-ip".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let result: Result<IpAllowlist, _> = config.try_into();
	assert!(result.is_err(), "invalid IP should fail to parse");
}

#[test]
fn test_invalid_cidr_in_config_fails() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["192.168.1.0/999".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let result: Result<IpAllowlist, _> = config.try_into();
	assert!(result.is_err(), "invalid CIDR should fail to parse");
}

// ==================== Serialization Tests ====================

#[test]
fn test_serde_roundtrip() {
	let config = IpAllowlistSerde {
		allowed_ips: vec![
			"192.168.1.0/24".to_string(),
			"10.0.0.1".to_string(),
			"2001:db8::/32".to_string(),
		],
		deny_status_code: 404,
		deny_message: Some("Custom message".to_string()),
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: -2,
	};

	// Serialize to JSON
	let json = serde_json::to_string(&config).unwrap();

	// Deserialize back
	let deserialized: IpAllowlistSerde = serde_json::from_str(&json).unwrap();

	assert_eq!(deserialized.allowed_ips, config.allowed_ips);
	assert_eq!(deserialized.deny_status_code, config.deny_status_code);
	assert_eq!(deserialized.deny_message, config.deny_message);
	assert_eq!(deserialized.ip_source, config.ip_source);
	assert_eq!(
		deserialized.distance_from_last_hop,
		config.distance_from_last_hop
	);
}

#[test]
fn test_default_deny_status_code() {
	let json = r#"{"allowedIps": ["10.0.0.1"]}"#;
	let config: IpAllowlistSerde = serde_json::from_str(json).unwrap();
	assert_eq!(
		config.deny_status_code, 403,
		"default status code should be 403"
	);
	assert_eq!(
		config.ip_source,
		IpSource::XForwardedFor,
		"default ip_source should be XForwardedFor"
	);
	assert_eq!(
		config.distance_from_last_hop, 0,
		"default distance should be 0"
	);
}

#[test]
fn test_policy_response_structure() {
	let config = IpAllowlistSerde {
		allowed_ips: vec!["10.0.0.1".to_string()],
		deny_status_code: 403,
		deny_message: None,
		ip_source: IpSource::RemoteAddr,
		distance_from_last_hop: 0,
	};
	let allowlist: IpAllowlist = config.try_into().unwrap();

	// Test allowed request returns empty PolicyResponse
	let mut req = request_with_ip("10.0.0.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_none());
	assert!(result.response_headers.is_none());

	// Test denied request returns PolicyResponse with direct_response
	let mut req = request_with_ip("192.168.1.1");
	let result = allowlist.apply(&mut req).unwrap();
	assert!(result.direct_response.is_some());
	assert!(result.response_headers.is_none());
}
