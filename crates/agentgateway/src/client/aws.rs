use aws_smithy_runtime_api::client::http::{
	HttpClient, HttpConnector, HttpConnectorFuture, HttpConnectorSettings, SharedHttpConnector,
};
use aws_smithy_runtime_api::client::orchestrator::{HttpRequest, HttpResponse};
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::body::SdkBody;

use crate::client::{Client, ConnectTimeout};
use crate::http::filters::BackendRequestTimeout;
use crate::*;

// Shim to put our Client type around AWS's client trait.
impl HttpClient for Client {
	fn http_connector(
		&self,
		settings: &HttpConnectorSettings,
		_components: &RuntimeComponents,
	) -> SharedHttpConnector {
		SharedHttpConnector::new(Connector {
			client: self.clone(),
			connect_timeout: settings.connect_timeout(),
			read_timeout: settings.read_timeout(),
		})
	}
}

#[derive(Debug)]
struct Connector {
	client: Client,
	connect_timeout: Option<Duration>,
	read_timeout: Option<Duration>,
}

impl HttpConnector for Connector {
	fn call(&self, request: HttpRequest) -> HttpConnectorFuture {
		let client = self.client.clone();
		let connect_timeout = self.connect_timeout;
		let read_timeout = self.read_timeout;
		HttpConnectorFuture::new(async move {
			let mut req = request
				.try_into_http1x()
				.map_err(|e| ConnectorError::other(e.into(), None))?
				.map(crate::http::Body::new);
			if let Some(t) = connect_timeout {
				req.extensions_mut().insert(ConnectTimeout(t));
			}
			// This bounds the entire request rather than each read; but credential responses are small so it is close enough.
			if let Some(t) = read_timeout {
				req.extensions_mut().insert(BackendRequestTimeout(t));
			}
			let rsp = client
				.simple_call(req)
				.await
				.map_err(|e| ConnectorError::io(e.into()))?;
			let (parts, body) = crate::http::read_response_body(rsp)
				.await
				.map_err(|e| ConnectorError::io(e.into()))?;
			HttpResponse::try_from(::http::Response::from_parts(parts, SdkBody::from(body)))
				.map_err(|e| ConnectorError::other(e.into(), None))
		})
	}
}
