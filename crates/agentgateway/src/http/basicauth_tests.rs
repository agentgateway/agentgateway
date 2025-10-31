use super::*;
use std::fs;
use std::io::Write;
use tempfile::NamedTempFile;

fn create_test_htpasswd() -> NamedTempFile {
	let mut file = NamedTempFile::new().unwrap();
	// Using bcrypt hash for password "test123"
	writeln!(file, "testuser:$2y$05$H5iJbsJPn0dZVD6kM6tOQuVJxLw7KjKCGvWlhG1SxNxLNn6hZoKYy").unwrap();
	// Using MD5 hash for password "password"
	writeln!(file, "admin:$apr1$Q/5qL8KZ$IZqKxM0kZQPsQqH9Lp9bL.").unwrap();
	file.flush().unwrap();
	file
}

#[test]
fn test_basic_auth_creation() {
	let file = create_test_htpasswd();
	let auth = BasicAuthentication::new(file.path().to_path_buf(), None);
	assert!(auth.is_ok());
	let auth = auth.unwrap();
	assert_eq!(auth.realm, "Restricted");
}

#[test]
fn test_basic_auth_custom_realm() {
	let file = create_test_htpasswd();
	let auth = BasicAuthentication::new(
		file.path().to_path_buf(),
		Some("Custom Realm".to_string()),
	);
	assert!(auth.is_ok());
	let auth = auth.unwrap();
	assert_eq!(auth.realm, "Custom Realm");
}

#[test]
fn test_basic_auth_nonexistent_file() {
	let auth = BasicAuthentication::new(
		std::path::PathBuf::from("/nonexistent/file"),
		None,
	);
	assert!(auth.is_err());
	assert!(matches!(auth.unwrap_err(), BasicAuthError::FileLoadError(_)));
}

#[tokio::test]
async fn test_valid_credentials() {
	let file = create_test_htpasswd();
	let mut auth = BasicAuthentication::new(file.path().to_path_buf(), None).unwrap();
	
	// Create a mock request with valid credentials
	let mut req = http::Request::builder()
		.uri("http://example.com")
		.header(
			"Authorization",
			"Basic dGVzdHVzZXI6dGVzdDEyMw==", // testuser:test123 base64 encoded
		)
		.body(axum::body::Body::empty())
		.unwrap();
	
	let mut log = crate::telemetry::log::RequestLog::default();
	let result = auth.apply(&mut log, &mut req).await;
	assert!(result.is_ok());
}

#[tokio::test]
async fn test_invalid_credentials() {
	let file = create_test_htpasswd();
	let mut auth = BasicAuthentication::new(file.path().to_path_buf(), None).unwrap();
	
	// Create a mock request with invalid credentials
	let mut req = http::Request::builder()
		.uri("http://example.com")
		.header(
			"Authorization",
			"Basic dGVzdHVzZXI6d3JvbmdwYXNz", // testuser:wrongpass base64 encoded
		)
		.body(axum::body::Body::empty())
		.unwrap();
	
	let mut log = crate::telemetry::log::RequestLog::default();
	let result = auth.apply(&mut log, &mut req).await;
	assert!(result.is_err());
}

#[tokio::test]
async fn test_missing_credentials() {
	let file = create_test_htpasswd();
	let mut auth = BasicAuthentication::new(file.path().to_path_buf(), None).unwrap();
	
	// Create a mock request without credentials
	let mut req = http::Request::builder()
		.uri("http://example.com")
		.body(axum::body::Body::empty())
		.unwrap();
	
	let mut log = crate::telemetry::log::RequestLog::default();
	let result = auth.apply(&mut log, &mut req).await;
	assert!(result.is_err());
}

#[test]
fn test_clone() {
	let file = create_test_htpasswd();
	let auth = BasicAuthentication::new(file.path().to_path_buf(), None).unwrap();
	let cloned = auth.clone();
	
	assert_eq!(auth.htpasswd_file, cloned.htpasswd_file);
	assert_eq!(auth.realm, cloned.realm);
	// htpasswd should not be cloned
	assert!(cloned.htpasswd.is_none());
}
