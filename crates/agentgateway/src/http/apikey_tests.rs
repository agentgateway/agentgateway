use super::*;

#[tokio::test]
async fn test_apikey_query_parameter_extracts_and_strips() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::QueryParameter {
			name: "api_key".into(),
		},
	);

	let mut req = ::http::Request::builder()
		.uri("http://example.com/data?api_key=test-api-key&keep=yes")
		.body(axum::body::Body::empty())
		.unwrap();

	auth.apply(&mut req).await.expect("api key should validate");

	assert_eq!(req.uri().to_string(), "http://example.com/data?keep=yes");
	assert!(req.extensions().get::<Claims>().is_some());
}

#[tokio::test]
async fn test_apikey_cookie_extracts_and_strips() {
	let auth = APIKeyAuthentication::new(
		[(APIKey::new("test-api-key"), serde_json::Value::Null)],
		Mode::Strict,
		AuthorizationLocation::Cookie {
			name: "api_key".into(),
		},
	);

	let mut req = ::http::Request::builder()
		.uri("http://example.com/data")
		.header("cookie", "keep=yes; api_key=test-api-key")
		.body(axum::body::Body::empty())
		.unwrap();

	auth.apply(&mut req).await.expect("api key should validate");

	assert_eq!(
		req.headers().get("cookie").unwrap().to_str().unwrap(),
		"keep=yes"
	);
	assert!(req.extensions().get::<Claims>().is_some());
}
