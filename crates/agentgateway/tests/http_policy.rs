use agentgateway::http::backend::{HTTP, HttpVersion};

#[test]
fn http_policy_version_override() {
    // Default (unset): no override
    let pol = HTTP { version: None };
    assert!(pol.version_override().is_none());

    // HTTP/1.1
    let pol = HTTP { version: Some(HttpVersion::Http1_1) };
    assert_eq!(pol.version_override(), Some(http::Version::HTTP_11));
    assert!(pol.is_http11());
    assert!(!pol.is_http2());

    // HTTP/2
    let pol = HTTP { version: Some(HttpVersion::Http2) };
    assert_eq!(pol.version_override(), Some(http::Version::HTTP_2));
    assert!(pol.is_http2());
    assert!(!pol.is_http11());
}

