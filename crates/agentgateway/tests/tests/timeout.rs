use crate::common::prelude::*;

/// Gap between the chunks written by [`collect_chunked_stream`]'s upstream. Shared by the tests
/// below so that the only difference between them is the policy under test.
const CHUNK_INTERVAL: Duration = Duration::from_millis(200);

/// Streams a chunked response through the proxy with `policy` attached, writing a chunk every
/// [`CHUNK_INTERVAL`]. Returns the body the client managed to read, plus the error the stream
/// terminated with if it did not end cleanly.
async fn collect_chunked_stream(policy: Value) -> (String, Option<String>) {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let upstream_address = listener.local_addr().unwrap();
	let upstream = tokio::spawn(async move {
		let (mut socket, _) = listener.accept().await.unwrap();
		let mut request = Vec::new();
		loop {
			let mut buffer = [0; 1024];
			let read = socket.read(&mut buffer).await.unwrap();
			assert_ne!(read, 0, "proxy closed before sending request headers");
			request.extend_from_slice(&buffer[..read]);
			if request.windows(4).any(|window| window == b"\r\n\r\n") {
				break;
			}
		}

		// The first chunk rides along with the headers, so it always reaches the client before any
		// idle window has had a chance to open.
		socket
			.write_all(
				b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n3\r\none\r\n",
			)
			.await
			.unwrap();
		// Once the gateway terminates the stream these writes fail; that is the case under test, so
		// the upstream just stops rather than asserting.
		for chunk in [b"3\r\ntwo\r\n".as_slice(), b"5\r\nthree\r\n".as_slice()] {
			tokio::time::sleep(CHUNK_INTERVAL).await;
			if socket.write_all(chunk).await.is_err() {
				return;
			}
		}
		tokio::time::sleep(CHUNK_INTERVAL).await;
		let _ = socket.write_all(b"0\r\n\r\n").await;
	});

	let mut bind = setup_proxy_test("{}")
		.unwrap()
		.with_backend(upstream_address)
		.with_bind(simple_bind())
		.with_route(basic_route(upstream_address));
	bind.attach_route_policy(policy).await;

	let io = bind.serve_http(BIND_KEY);
	let response = RequestBuilder::new(Method::GET, "http://lo/")
		.send(io)
		.await
		.expect("response headers should arrive before any idle window elapses");
	assert_eq!(response.status(), 200);

	let mut body = response.into_body();
	let mut collected = String::new();
	let mut error = None;
	let read = async {
		while let Some(frame) = body.frame().await {
			match frame {
				Ok(frame) => {
					if let Some(data) = frame.data_ref() {
						collected.push_str(&String::from_utf8_lossy(data));
					}
				},
				Err(e) => {
					error = Some(e.to_string());
					break;
				},
			}
		}
	};
	tokio::time::timeout(Duration::from_secs(5), read)
		.await
		.expect("stream should terminate rather than hang");

	upstream.abort();
	(collected, error)
}

/// A stream that keeps producing data survives an idle window shorter than the total response
/// duration, and `requestTimeout` does not bound the response body.
#[tokio::test]
async fn response_activity_restarts_idle_window_across_response_chunks() {
	let (body, error) = collect_chunked_stream(json!({
		"timeout": {
			"requestTimeout": "200ms",
			"responseIdleTimeout": "1s"
		}
	}))
	.await;
	assert_eq!(
		error, None,
		"chunks arriving inside the idle window should keep the stream alive"
	);
	assert_eq!(body, "onetwothree");
}

/// Negative control for the test above: the same chunk cadence is terminated once the idle window
/// is shorter than the gap between chunks. Without this, the test above would also pass with no
/// `responseIdleTimeout` set at all, since nothing else bounds a streaming response body.
#[tokio::test]
async fn response_idle_timeout_terminates_slow_response_chunks() {
	let (body, error) = collect_chunked_stream(json!({
		"timeout": {
			"requestTimeout": "1s",
			"responseIdleTimeout": "100ms"
		}
	}))
	.await;
	// The client only ever observes a broken stream here: the response status was already sent, so
	// the gateway can do no more than drop the body.
	assert!(
		error.is_some(),
		"chunks slower than the idle window should terminate the stream, got {body:?}"
	);
	// Pins down *when* it was terminated. The first chunk arrives with the headers; the second is
	// due CHUNK_INTERVAL later, well past the window. Asserting only that the stream broke would
	// also accept it breaking for an unrelated reason.
	assert_eq!(body, "one");
}

/// A zero duration disables the timeout rather than expiring immediately, matching how the Gateway
/// API HTTPRoute translation treats `request`/`backendRequest`.
#[tokio::test]
async fn zero_response_idle_timeout_disables_the_timeout() {
	let (body, error) = collect_chunked_stream(json!({
		"timeout": {
			"responseIdleTimeout": "0s"
		}
	}))
	.await;
	assert_eq!(
		error, None,
		"a zero idle timeout should not terminate the stream"
	);
	assert_eq!(body, "onetwothree");
}
