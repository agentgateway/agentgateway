use http::{Method, StatusCode};
use wiremock::{Mock, ResponseTemplate};

use crate::common::gateway::AgentGateway;

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
