pub mod gateway;
pub mod hbone_server;
pub mod mock_ca_server;
pub mod shared_ca;

pub mod prelude {
	pub type MemoryClient = hyper_util::client::legacy::Client<MemoryConnector, Body>;
	pub type StdMutex<T> = std::sync::Mutex<T>;
	pub use std::assert_matches;
	pub use std::convert::Infallible;
	pub use std::sync::Arc;
	pub use std::sync::atomic::{AtomicUsize, Ordering};
	pub use std::time::Duration;

	pub use agent_core::strng;
	pub use agent_pool::rt::TokioExecutor;
	pub use agentgateway::http::tests_common::ResponseExt;
	pub use agentgateway::http::{Body, Response};
	pub use agentgateway::proxy::request_builder::RequestBuilder;
	pub use agentgateway::read_body;
	pub use agentgateway::test_helpers::proxymock::*;
	pub use agentgateway::types::agent::{
		Backend, BackendTrafficPolicy, BackendWithPolicies, Bind, BindProtocol, Listener,
		ListenerProtocol, ListenerSet, PathMatch, ResourceName, Route, RouteMatch,
		SimpleBackendReference, Target,
	};
	pub use agentgateway::types::backend;
	pub use http::{HeaderMap, Method, StatusCode, Version, header};
	pub use http_body::Frame;
	pub use http_body_util::{BodyExt, StreamBody};
	pub use hyper::service::service_fn;
	pub use hyper_util::rt::TokioIo;
	pub use rand::RngExt;
	pub use serde::Serialize;
	pub use serde_json::{Value, json};
	pub use tokio::io::{AsyncReadExt, AsyncWriteExt};
	pub use tokio::net::{TcpListener, TcpStream};
	pub use tokio::sync::oneshot;
	pub use url::Url;
	pub use wiremock::{Mock, MockServer, ResponseTemplate};
}
