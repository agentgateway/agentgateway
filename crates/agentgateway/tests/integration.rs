use http::{Method, StatusCode};
use wiremock::{Mock, ResponseTemplate};

mod common;
use common::compare::*;
use common::gateway::*;

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

#[tokio::test]
async fn test_basic_routes() -> anyhow::Result<()> {
	let mock = wiremock::MockServer::start().await;
	Mock::given(wiremock::matchers::path_regex("/.*"))
		.respond_with(move |_: &wiremock::Request| ResponseTemplate::new(200))
		.mount(&mock)
		.await;
	let gw = AgentGateway::new(format!(
		r#"config: {{}}
binds:
- port: $PORT
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: default
      policies:
        urlRewrite:
          path:
            prefix: /xxxx
        transformations:
          request:
          response:
            add:
              x-resp: '"foo"'
      backends:
        - host: {}
"#,
		mock.address()
	))
	.await?;
	let resp = gw.send_request(Method::GET, "http://localhost").await;
	assert_eq!(resp.status(), StatusCode::OK);
	let rh = resp.headers().get("x-resp").unwrap();
	assert_eq!(rh.to_str().unwrap(), "foo");
	Ok(())
}
