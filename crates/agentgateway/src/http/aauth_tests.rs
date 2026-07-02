use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use http_message_sig::headers::{
	SignatureParams, build_signature, build_signature_input, build_signature_key_hwk,
	build_signature_key_jwks,
};
use http_message_sig::keys::ed25519::{
	PrivateKey, generate_keypair, public_key_to_base64url, sign,
};
use http_message_sig::keys::jwk::JWK;
use http_message_sig::signing::build_signature_base;

use super::*;
use crate::http::Body;
use crate::test_helpers::test_policy;

fn now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.unwrap()
		.as_secs()
}

/// Build a fully signed request against the given URL using the supplied Signature-Key header
/// value and `created` timestamp. The scheme is encoded in `sig_key_header` (hwk vs jwks_uri vs
/// jwt); everything below the scheme — sig base, signing, header attachment — is identical.
fn build_signed_request_with_sig_key(
	method: &str,
	url: &str,
	sig_key_header: &str,
	private_key: &PrivateKey,
	created: u64,
) -> Request {
	let parsed = url::Url::parse(url).expect("test URL must parse");
	let authority = parsed.host_str().unwrap();
	let port = parsed.port();
	let signed_authority = match port {
		Some(p) => format!("{}:{}", authority, p),
		None => authority.to_string(),
	};

	let params = SignatureParams {
		created,
		keyid: None,
		nonce: None,
		alg: None,
	};
	let components = ["@method", "@authority", "@path", "signature-key"];

	let mut headers = HashMap::new();
	headers.insert("Signature-Key".to_string(), sig_key_header.to_string());

	let base = build_signature_base(
		method,
		&signed_authority,
		parsed.path(),
		parsed.query(),
		&headers,
		&components,
		&params,
	)
	.unwrap();
	let signature_bytes = sign(base.as_bytes(), private_key);

	::http::Request::builder()
		.method(method)
		.uri(url)
		.header("Signature-Key", sig_key_header)
		.header(
			"Signature-Input",
			build_signature_input("sig1", &components, &params),
		)
		.header("Signature", build_signature("sig1", &signature_bytes))
		.body(Body::empty())
		.unwrap()
}

fn hwk_jwk_for(public_key: &http_message_sig::keys::ed25519::PublicKey) -> JWK {
	JWK {
		kty: "OKP".to_string(),
		crv: Some("Ed25519".to_string()),
		x: Some(public_key_to_base64url(public_key)),
		y: None,
		d: None,
		n: None,
		e: None,
		kid: None,
		alg: None,
		extra: Default::default(),
	}
}

/// Build a fully signed GET request against the given URL using the hwk scheme. Returns the
/// request and the signing keypair.
fn build_signed_request(method: &str, url: &str) -> (Request, PrivateKey) {
	build_signed_request_at(method, url, now())
}

/// Like [`build_signed_request`] but with a caller-supplied `created` timestamp. Used to exercise
/// the freshness window.
fn build_signed_request_at(method: &str, url: &str, created: u64) -> (Request, PrivateKey) {
	let (private_key, public_key) = generate_keypair();
	let sig_key_header = build_signature_key_hwk("sig1", &hwk_jwk_for(&public_key)).unwrap();
	let req = build_signed_request_with_sig_key(method, url, &sig_key_header, &private_key, created);
	(req, private_key)
}

/// Build a request signed under the `jwks_uri` scheme. The caller is responsible for seeding the
/// policy's `JwksCache` with the returned JWK under `(id, dwk)` so the verifier finds it without
/// performing a network fetch.
fn build_jwks_signed_request(
	method: &str,
	url: &str,
	id: &str,
	dwk: &str,
	kid: &str,
) -> (Request, JWK) {
	let (private_key, public_key) = generate_keypair();
	let mut jwk = hwk_jwk_for(&public_key);
	jwk.kid = Some(kid.to_string());
	let sig_key_header = build_signature_key_jwks("sig1", id, kid, dwk);
	let req = build_signed_request_with_sig_key(method, url, &sig_key_header, &private_key, now());
	(req, jwk)
}

fn empty_request(method: &str, url: &str) -> Request {
	::http::Request::builder()
		.method(method)
		.uri(url)
		.body(Body::empty())
		.unwrap()
}

#[tokio::test]
async fn strict_mode_rejects_request_without_signature() {
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Hwk, 60, false);
	let mut req = empty_request("GET", "https://example.com/api/data");

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	// Strict mode + no signature → policy yields a direct 401 response.
	let resp = result.direct_response.expect("expected direct response");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn optional_mode_allows_request_without_signature() {
	let policy = AAuth::new(Mode::Optional, RequiredScheme::Hwk, 60, false);
	let mut req = empty_request("GET", "https://example.com/api/data");

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	assert!(result.direct_response.is_none());
	assert!(req.extensions().get::<AAuthClaims>().is_none());
}

#[tokio::test]
async fn permissive_mode_lets_invalid_signature_through() {
	let policy = AAuth::new(Mode::Permissive, RequiredScheme::Hwk, 60, false);
	// Send only Signature-Key — Signature-Input/Signature missing → counts as missing signature.
	let mut req = ::http::Request::builder()
		.method("GET")
		.uri("https://example.com/api/data")
		.header("Signature-Key", r#"sig1=hwk;kty="OKP""#)
		.body(Body::empty())
		.unwrap();

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	assert!(result.direct_response.is_none());
}

#[tokio::test]
async fn strict_mode_accepts_valid_hwk_signature() {
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Hwk, 60, false);
	let (mut req, _key) = build_signed_request("GET", "https://example.com/api/data");

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	assert!(
		result.direct_response.is_none(),
		"expected accept, got {:?}",
		result.direct_response.as_ref().map(|r| r.status())
	);
	let claims = req
		.extensions()
		.get::<AAuthClaims>()
		.expect("AAuthClaims should be attached");
	assert_eq!(
		claims.inner.get("scheme").and_then(|v| v.as_str()),
		Some("hwk")
	);
	// hwk has no `agent` (pseudonymous).
	assert!(claims.inner.get("agent").is_none());
	// Thumbprint should be populated from the inline JWK.
	assert!(
		claims
			.inner
			.get("thumbprint")
			.and_then(|v| v.as_str())
			.is_some_and(|s| !s.is_empty())
	);
}

#[tokio::test]
async fn strict_mode_rejects_tampered_signature() {
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Hwk, 60, false);
	let (mut req, _key) = build_signed_request("GET", "https://example.com/api/data");

	// Tamper: swap the path the verifier reconstructs against. The signature was over /api/data
	// so verifying as /other will fail.
	*req.uri_mut() = "https://example.com/other".parse().unwrap();

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	let resp = result.direct_response.expect("expected rejection");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn strict_mode_rejects_signed_request_below_required_scheme() {
	// hwk presented, but we require agentJwt. Should yield 401 with the spec-defined
	// `AAuth-Requirement: requirement=agent-token` challenge header — agent identity is
	// what the gateway is asking for, not authorization.
	let policy = AAuth::new(Mode::Strict, RequiredScheme::AgentJwt, 60, false);
	let (mut req, _key) = build_signed_request("GET", "https://example.com/api/data");

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	let resp = result.direct_response.expect("expected rejection");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
	let challenge = resp
		.headers()
		.get("AAuth-Requirement")
		.expect("AAuth-Requirement header expected on insufficient level")
		.to_str()
		.unwrap();
	assert_eq!(challenge, "requirement=agent-token");
}

#[tokio::test]
async fn strict_mode_rejects_signature_outside_tolerance() {
	// Signature is otherwise valid but `created` is older than the tolerance window. The
	// freshness check (verifier `time_diff > tolerance`) must reject it. Paired with
	// `strict_mode_accepts_valid_hwk_signature` (created = now), which exercises the
	// within-window path — together they establish that it's the timestamp specifically
	// driving the rejection.
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Hwk, 60, false);
	let stale = now() - 120;
	let (mut req, _key) = build_signed_request_at("GET", "https://example.com/api/data", stale);

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	let resp = result.direct_response.expect("expected rejection");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
	// No claims attached on a rejection.
	assert!(req.extensions().get::<AAuthClaims>().is_none());
}

#[tokio::test]
async fn strict_mode_accepts_valid_jwks_uri_signature() {
	// End-to-end coverage of the `jwks_uri` scheme without spinning up an HTTP issuer: pre-seed
	// the policy's JwksCache so `get_jwks_key` short-circuits on the cache hit, then verify the
	// full apply path (parse Signature-Key → resolve key via cache → Ed25519 verify → emit
	// claims) succeeds and populates aauth.* claims correctly.
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Jwks, 60, false);
	let issuer = "https://issuer.example";
	let dwk = "aauth-agent.json";
	let kid = "key-1";

	let (mut req, jwk) =
		build_jwks_signed_request("GET", "https://example.com/api/data", issuer, dwk, kid);
	policy
		.jwks_cache
		.insert(issuer, dwk, std::slice::from_ref(&jwk));

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	assert!(
		result.direct_response.is_none(),
		"expected accept, got {:?}",
		result.direct_response.as_ref().map(|r| r.status())
	);
	let claims = req
		.extensions()
		.get::<AAuthClaims>()
		.expect("AAuthClaims should be attached");
	assert_eq!(
		claims.inner.get("scheme").and_then(|v| v.as_str()),
		Some("jwks_uri")
	);
	// jwks_uri carries identified agent identity — the `id` parameter from Signature-Key.
	assert_eq!(
		claims.inner.get("agent").and_then(|v| v.as_str()),
		Some(issuer)
	);
}

#[tokio::test]
async fn strict_mode_rejects_jwks_uri_with_unknown_kid() {
	// The signed request claims `kid="key-1"` but the cache has only `kid="other"` under
	// (issuer, dwk). Verifier should fail to resolve the key — there's no network fetch
	// attempt because the cache is warm — and reject with 401. Without this test, a stale
	// cache entry with overlapping kids would mask a misconfiguration.
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Jwks, 60, false);
	let issuer = "https://issuer.example";
	let dwk = "aauth-agent.json";

	let (mut req, _signing_jwk) =
		build_jwks_signed_request("GET", "https://example.com/api/data", issuer, dwk, "key-1");
	// Seed the cache with a different key under the same (id, dwk) but under a different kid.
	let (_pk, decoy_pub) = generate_keypair();
	let mut decoy_jwk = hwk_jwk_for(&decoy_pub);
	decoy_jwk.kid = Some("other".to_string());
	policy
		.jwks_cache
		.insert(issuer, dwk, std::slice::from_ref(&decoy_jwk));

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	let resp = result.direct_response.expect("expected rejection");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
	assert!(req.extensions().get::<AAuthClaims>().is_none());
}

#[tokio::test]
async fn strict_mode_rejects_jwks_uri_when_cached_key_does_not_match_signature() {
	// kid matches but the cached public key belongs to a different keypair than the one that
	// signed the request. Cache lookup succeeds; Ed25519 verification must then fail. Guards
	// against a bug where the verifier silently accepts whatever key the cache hands back
	// without actually checking the signature against it.
	let policy = AAuth::new(Mode::Strict, RequiredScheme::Jwks, 60, false);
	let issuer = "https://issuer.example";
	let dwk = "aauth-agent.json";
	let kid = "key-1";

	let (mut req, _signing_jwk) =
		build_jwks_signed_request("GET", "https://example.com/api/data", issuer, dwk, kid);
	// Seed the cache under the SAME kid but with a freshly generated, unrelated keypair.
	let (_pk, decoy_pub) = generate_keypair();
	let mut decoy_jwk = hwk_jwk_for(&decoy_pub);
	decoy_jwk.kid = Some(kid.to_string());
	policy
		.jwks_cache
		.insert(issuer, dwk, std::slice::from_ref(&decoy_jwk));

	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	let resp = result.direct_response.expect("expected rejection");
	assert_eq!(resp.status(), ::http::StatusCode::UNAUTHORIZED);
	assert!(req.extensions().get::<AAuthClaims>().is_none());
}

#[tokio::test]
async fn permissive_mode_lets_tampered_signature_through() {
	let policy = AAuth::new(Mode::Permissive, RequiredScheme::Hwk, 60, false);
	let (mut req, _key) = build_signed_request("GET", "https://example.com/api/data");

	*req.uri_mut() = "https://example.com/other".parse().unwrap();
	let result = test_policy(&policy, &mut req).await.expect("policy ran");
	assert!(result.direct_response.is_none());
	// No claims because we didn't trust the signature.
	assert!(req.extensions().get::<AAuthClaims>().is_none());
}

#[test]
fn required_scheme_ordering() {
	use http_message_sig::signing::SignatureScheme;

	// Hwk accepts anything.
	assert!(RequiredScheme::Hwk.allows(&SignatureScheme::Hwk));
	assert!(RequiredScheme::Hwk.allows(&SignatureScheme::Jwks));
	assert!(RequiredScheme::Hwk.allows(&SignatureScheme::Jwt));

	// Jwks requires identified scheme or stronger.
	assert!(!RequiredScheme::Jwks.allows(&SignatureScheme::Hwk));
	assert!(RequiredScheme::Jwks.allows(&SignatureScheme::Jwks));
	assert!(RequiredScheme::Jwks.allows(&SignatureScheme::Jwt));

	// AgentJwt requires a JWT-bound key. Both aa-agent+jwt and aa-auth+jwt carry verified
	// agent identity, so either satisfies the requirement; the gateway does not require
	// the stronger "authorization token" semantics (requirement=auth-token).
	assert!(!RequiredScheme::AgentJwt.allows(&SignatureScheme::Hwk));
	assert!(!RequiredScheme::AgentJwt.allows(&SignatureScheme::Jwks));
	assert!(RequiredScheme::AgentJwt.allows(&SignatureScheme::Jwt));
}

#[test]
fn challenge_response_strings() {
	// Spec-aligned: RFC 8941 Structured Field Dictionary with `requirement=<token>` key,
	// emitted under the `AAuth-Requirement` header. See draft-hardt-oauth-aauth-protocol §6.
	let hwk = AAuth::new(Mode::Strict, RequiredScheme::Hwk, 60, false);
	assert_eq!(hwk.build_challenge_response(), "requirement=pseudonym");

	let jwks = AAuth::new(Mode::Strict, RequiredScheme::Jwks, 60, false);
	assert_eq!(jwks.build_challenge_response(), "requirement=identity");

	let agent_jwt = AAuth::new(Mode::Strict, RequiredScheme::AgentJwt, 60, false);
	assert_eq!(
		agent_jwt.build_challenge_response(),
		"requirement=agent-token"
	);
}

#[test]
fn signature_header_snapshot_combines_repeated_fields() {
	let mut headers = ::http::HeaderMap::new();
	headers.append("authorization", "AAuth first".parse().unwrap());
	headers.append("authorization", "AAuth second".parse().unwrap());
	headers.append("x-test", "  one  ".parse().unwrap());
	headers.append("x-test", "two".parse().unwrap());

	let snapshot = super::snapshot_headers_for_signature(&headers);
	assert_eq!(
		snapshot.get("authorization").map(String::as_str),
		Some("AAuth first, AAuth second")
	);
	assert_eq!(snapshot.get("x-test").map(String::as_str), Some("one, two"));
}

fn jwk_with_kid(kid: &str) -> JWK {
	JWK {
		kty: "OKP".to_string(),
		crv: Some("Ed25519".to_string()),
		x: Some("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs".to_string()),
		y: None,
		d: None,
		n: None,
		e: None,
		kid: Some(kid.to_string()),
		alg: None,
		extra: serde_json::Map::new(),
	}
}

#[test]
fn jwks_cache_get_after_insert() {
	let cache = JwksCache::default();
	let jwk = jwk_with_kid("key-1");
	cache.insert(
		"https://agent.example.com",
		"aauth-agent.json",
		std::slice::from_ref(&jwk),
	);
	let retrieved = cache
		.get("https://agent.example.com", "aauth-agent.json", "key-1")
		.unwrap();
	assert_eq!(retrieved.kid.as_deref(), Some("key-1"));
	assert!(
		cache
			.get("https://agent.example.com", "aauth-agent.json", "missing")
			.is_none()
	);
	assert!(
		cache
			.get("https://other.example.com", "aauth-agent.json", "key-1")
			.is_none()
	);
}

#[test]
fn jwks_cache_evicts_stale_entries_lazily() {
	// Without lazy eviction, expired entries would accumulate as unique (id, dwk) pairs
	// rotate. After TTL the entry should be gone from the map entirely, not just hidden.
	let cache = JwksCache::default();
	let issuer = "https://example.com";
	let jwk = jwk_with_kid("key-1");
	cache.insert(issuer, "aauth-agent.json", std::slice::from_ref(&jwk));
	assert_eq!(cache.entry_count(), 1);

	// Backdate the entry well past TTL.
	cache.backdate(
		issuer,
		"aauth-agent.json",
		std::time::Duration::from_secs(60 * 60),
	);

	// First `get` returns None AND evicts.
	assert!(cache.get(issuer, "aauth-agent.json", "key-1").is_none());
	assert_eq!(
		cache.entry_count(),
		0,
		"stale entry should have been removed",
	);
}

#[test]
fn jwks_cache_distinguishes_by_dwk() {
	// Regression: the cache used to be keyed by issuer id alone. Two routes against the same
	// issuer with distinct dwk discovery documents (e.g. aauth-agent.json vs aauth-issuer.json)
	// can publish disjoint key sets — sharing across dwk values aliased them. Now keyed by
	// (id, dwk).
	let cache = JwksCache::default();
	let issuer = "https://example.com";
	let agent_key = jwk_with_kid("agent-1");
	let auth_key = jwk_with_kid("auth-1");

	cache.insert(issuer, "aauth-agent.json", std::slice::from_ref(&agent_key));
	cache.insert(issuer, "aauth-issuer.json", std::slice::from_ref(&auth_key));

	assert_eq!(
		cache
			.get(issuer, "aauth-agent.json", "agent-1")
			.unwrap()
			.kid
			.as_deref(),
		Some("agent-1"),
	);
	assert_eq!(
		cache
			.get(issuer, "aauth-issuer.json", "auth-1")
			.unwrap()
			.kid
			.as_deref(),
		Some("auth-1"),
	);
	// The auth dwk's keys must NOT leak into the agent dwk's lookup, and vice versa.
	assert!(cache.get(issuer, "aauth-agent.json", "auth-1").is_none());
	assert!(cache.get(issuer, "aauth-issuer.json", "agent-1").is_none());
}

#[test]
fn validate_jwks_uri_accepts_https() {
	assert!(super::validate_jwks_uri("https://issuer/jwks.json", "https://issuer", false).is_ok());
	assert!(
		super::validate_jwks_uri(
			"https://issuer:8443/jwks.json",
			"https://issuer:8443",
			false
		)
		.is_ok()
	);
}

#[test]
fn validate_jwks_uri_rejects_http_by_default() {
	let err =
		super::validate_jwks_uri("http://attacker/jwks.json", "http://attacker", false).unwrap_err();
	match err {
		super::Error::InvalidSignature { description, .. } => {
			assert!(
				description.contains("must use https"),
				"unexpected description: {description}",
			);
		},
		other => panic!("expected InvalidSignature, got: {other}"),
	}
}

#[test]
fn validate_jwks_uri_allows_http_loopback_under_dev_flag() {
	assert!(
		super::validate_jwks_uri(
			"http://localhost:8080/jwks.json",
			"http://localhost:8080",
			true
		)
		.is_ok()
	);
	assert!(super::validate_jwks_uri("http://127.0.0.1/jwks.json", "http://127.0.0.1", true).is_ok());
	assert!(
		super::validate_jwks_uri(
			"http://127.5.6.7:9099/jwks.json",
			"http://127.5.6.7:9099",
			true
		)
		.is_ok()
	);
	assert!(super::validate_jwks_uri("http://[::1]/jwks.json", "http://[::1]", true).is_ok());
}

#[test]
fn validate_jwks_uri_rejects_non_loopback_http_even_under_dev_flag() {
	// Critical: the dev flag MUST NOT escalate into "fetch keys from any HTTP host". An
	// attacker who controls the issuer metadata could otherwise return
	// jwks_uri=http://attacker.com/keys and downgrade the transport for the signing keys.
	for url in [
		"http://attacker.com/jwks.json",
		"http://192.168.1.5/jwks.json",
		"http://10.0.0.5:8080/jwks.json",
	] {
		let err = super::validate_jwks_uri(url, "http://attacker.com", true).unwrap_err();
		match err {
			super::Error::InvalidSignature { description, .. } => assert!(
				description.contains("loopback"),
				"expected loopback rejection, got: {description}",
			),
			other => panic!("expected InvalidSignature, got {other}"),
		}
	}
}

#[test]
fn validate_jwks_uri_rejects_garbage() {
	assert!(super::validate_jwks_uri("not a url", "https://issuer", false).is_err());
	assert!(super::validate_jwks_uri("ftp://issuer/keys", "https://issuer", true).is_err());
}

#[test]
fn validate_jwks_uri_rejects_cross_origin() {
	let err = super::validate_jwks_uri(
		"https://keys.example/jwks.json",
		"https://issuer.example",
		false,
	)
	.unwrap_err();
	match err {
		super::Error::InvalidSignature { description, .. } => assert!(
			description.contains("cross-origin"),
			"expected cross-origin rejection, got: {description}",
		),
		other => panic!("expected InvalidSignature, got {other}"),
	}
}

#[test]
fn validate_discovery_inputs_reject_path_and_dwk_traversal() {
	assert!(super::validate_discovery_id("https://issuer.example", false).is_ok());
	assert!(super::validate_discovery_id("https://issuer.example/path", false).is_err());
	assert!(super::validate_discovery_id("http://issuer.example", false).is_err());
	assert!(super::validate_dwk("aauth-agent.json").is_ok());
	assert!(super::validate_dwk("../secret").is_err());
	assert!(super::validate_dwk("nested/config").is_err());
}

#[test]
fn local_aauth_config_deserialize_minimal() {
	let yaml = r#"
mode: strict
requiredScheme: hwk
"#;
	let cfg: LocalAAuthConfig = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(cfg.timestamp_tolerance, 60);
	assert!(!cfg.allow_insecure_http_issuer);
}

#[test]
fn local_aauth_config_deserialize_full() {
	let yaml = r#"
mode: optional
requiredScheme: agentJwt
timestampTolerance: 30
allowInsecureHttpIssuer: true
"#;
	let cfg: LocalAAuthConfig = serde_yaml::from_str(yaml).unwrap();
	assert_eq!(cfg.timestamp_tolerance, 30);
	assert!(cfg.allow_insecure_http_issuer);
}

#[test]
fn local_aauth_config_rejects_unknown_field() {
	let yaml = r#"
mode: strict
requiredScheme: hwk
unknownField: nope
"#;
	let err = serde_yaml::from_str::<LocalAAuthConfig>(yaml).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("unknownField") || msg.contains("unknown field"),
		"expected deny_unknown_fields error, got: {}",
		msg
	);
}
