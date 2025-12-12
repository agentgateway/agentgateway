use wiremock::{Mock, ResponseTemplate};

use crate::common::compare::ProxyComparisonTest;

#[tokio::test]
async fn test_basic_proxy_comparison() -> anyhow::Result<()> {
	agent_core::telemetry::testing::setup_test_logging();
	if !ProxyComparisonTest::should_run() {
		return Ok(());
	}
	// Set up the test framework
	let test = ProxyComparisonTest::new().await?;
	// Configure the backend to return a simple response
	Mock::given(wiremock::matchers::method("GET"))
		.and(wiremock::matchers::path("/test"))
		.respond_with(
			ResponseTemplate::new(200)
				.set_body_string("Hello, World!")
				.insert_header("content-type", "text/plain"),
		)
		.mount(&test.backend_server)
		.await;

	// Send the same request to both proxies
	let comparison = test.compare_request("GET", "/test", None, None).await?;

	// Assert they behave identically
	comparison.assert_identical()?;

	Ok(())
}
