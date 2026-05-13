use super::*;
use crate::http::auth::AuthorizationLocation;

fn create_test_htpasswd() -> String {
	r#"testuser:$2y$05$rhjrEU0aFts7v/4WVz20uOlkI3eekXwvBRV6Q3TcYX46DhOhC42au
admin:$apr1$Q/5qL8KZ$IZqKxM0kZQPsQqH9Lp9bL.
	"#
	.to_string()
}

#[tokio::test]
async fn test_valid_credentials() {
	let auth = BasicAuthentication::new(
		&create_test_htpasswd(),
		None,
		Mode::Strict,
		super::default_authorization_location(),
	);

	// Create a mock request with valid credentials
	let mut req = ::http::Request::builder()
		.uri("http://example.com")
		.header(
			"Authorization",
			"Basic dGVzdHVzZXI6dGVzdDEyMw==", // testuser:test123 base64 encoded
		)
		.body(axum::body::Body::empty())
		.unwrap();

	let result = auth.verify(&mut req).await;
	assert!(result.is_ok());
}

#[tokio::test]
async fn test_invalid_credentials_strict_mode() {
	let auth = BasicAuthentication::new(
		&create_test_htpasswd(),
		None,
		Mode::Strict,
		super::default_authorization_location(),
	);

	// Create a mock request with invalid credentials
	let mut req = ::http::Request::builder()
		.uri("http://example.com")
		.header(
			"Authorization",
			"Basic dGVzdHVzZXI6d3JvbmdwYXNz", // testuser:wrongpass base64 encoded
		)
		.body(axum::body::Body::empty())
		.unwrap();

	let result = auth.verify(&mut req).await;
	assert!(result.is_err());
}

#[tokio::test]
async fn test_missing_credentials_strict_mode() {
	let auth = BasicAuthentication::new(
		&create_test_htpasswd(),
		None,
		Mode::Strict,
		super::default_authorization_location(),
	);

	// Create a mock request without credentials
	let mut req = ::http::Request::builder()
		.uri("http://example.com")
		.body(axum::body::Body::empty())
		.unwrap();

	let result = auth.verify(&mut req).await;
	assert!(result.is_err());
}

#[tokio::test]
async fn test_missing_credentials_optional_mode() {
	let auth = BasicAuthentication::new(
		&create_test_htpasswd(),
		None,
		Mode::Optional,
		super::default_authorization_location(),
	);

	// Create a mock request without credentials
	let mut req = ::http::Request::builder()
		.uri("http://example.com")
		.body(axum::body::Body::empty())
		.unwrap();

	let result = auth.verify(&mut req).await;
	// Should succeed in optional mode when no credentials provided
	assert!(result.is_ok());
}

#[tokio::test]
async fn test_query_parameter_credentials() {
	let auth = BasicAuthentication::new(
		&create_test_htpasswd(),
		None,
		Mode::Strict,
		AuthorizationLocation::QueryParameter {
			name: "auth".into(),
		},
	);

	let mut req = ::http::Request::builder()
		.uri("http://example.com?auth=dGVzdHVzZXI6dGVzdDEyMw==&keep=yes")
		.body(axum::body::Body::empty())
		.unwrap();

	let _ = crate::test_helpers::test_policy(&auth, &mut req)
		.await
		.expect("basic auth should validate");
	assert_eq!(req.uri().to_string(), "http://example.com/?keep=yes");
	assert!(req.extensions().get::<Claims>().is_some());
}
