use agentgateway::types::backend::HTTP;

#[test]
fn http_policy_is_http11() {
    // Default (unset): no version
    let pol = HTTP { version: None };
    assert!(!pol.is_http11());

    // HTTP/1.1
    let pol = HTTP { version: Some(http::Version::HTTP_11) };
    assert!(pol.is_http11());

    // HTTP/2
    let pol = HTTP { version: Some(http::Version::HTTP_2) };
    assert!(!pol.is_http11());
}
