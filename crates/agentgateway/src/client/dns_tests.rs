use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use assert_matches::assert_matches;

use super::*;

#[derive(Debug)]
pub(super) struct Mock {
	#[allow(clippy::type_complexity)]
	responses: Mutex<HashMap<String, (Box<[IpAddr]>, Instant)>>,
}

impl Mock {
	pub fn new() -> Self {
		Self {
			responses: Mutex::new(HashMap::new()),
		}
	}

	pub fn add_response(&self, host: &str, ips: Vec<IpAddr>, ttl_secs: u64) {
		let expiry = Instant::now() + Duration::from_secs(ttl_secs);
		let mut responses = self.responses.lock().unwrap();
		responses.insert(host.to_string(), (ips.into_boxed_slice(), expiry));
	}

	pub async fn resolve(&self, host: &str) -> Result<(Box<[IpAddr]>, Instant), ResolveError> {
		let responses = self.responses.lock().unwrap();
		responses
			.get(host)
			.cloned()
			.ok_or_else(|| ResolveError::from("host not found"))
	}
}

const IP1: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
const IP2: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));
const IP3: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 3));

#[tokio::test]
async fn test_basic_resolution() {
	let mock = Arc::new(Mock::new());
	mock.add_response("example.com", vec![IP1, IP2], 60);

	let resolver = CachedResolver {
		dns: Arc::new(Resolver::Mock(mock)),
		entries: Arc::new(Mutex::new(HashMap::new())),
	};

	// First resolution should work
	let ip1 = resolver.resolve("example.com".into()).await.unwrap();
	let ip2 = resolver.resolve("example.com".into()).await.unwrap();

	// Should get different IPs due to round-robin
	assert_ne!(ip1, ip2);

	// Third call should cycle back to first IP
	let ip3 = resolver.resolve("example.com".into()).await.unwrap();
	assert_eq!(ip1, ip3);
}

#[tokio::test(start_paused = true)]
async fn test_ip_change() {
	agent_core::telemetry::testing::setup_test_logging();
	let mock = Arc::new(Mock::new());
	mock.add_response("example.com", vec![IP1, IP2], 60);

	let resolver = CachedResolver {
		dns: Arc::new(Resolver::Mock(mock.clone())),
		entries: Arc::new(Mutex::new(HashMap::new())),
	};

	// First resolution should work
	let _ = resolver.resolve("example.com".into()).await.unwrap();
	let _ = resolver.resolve("example.com".into()).await.unwrap();
	mock.add_response("example.com", vec![IP3], 60);
	tokio::time::sleep(Duration::from_secs(30)).await;
	// Only some time has passed, cycle back to the first one
	assert_eq!(resolver.resolve("example.com".into()).await.unwrap(), IP1);
	tokio::time::sleep(Duration::from_secs(31)).await;

	// Now we should refresh and get new IPs
	assert_eq!(resolver.resolve("example.com".into()).await.unwrap(), IP3);

	mock.add_response("example.com", vec![], 60);
	tokio::time::sleep(Duration::from_secs(61)).await;
	assert_matches!(resolver.resolve("example.com".into()).await, Err(_));
}

#[tokio::test(start_paused = true)]
async fn test_ip_error() {
	agent_core::telemetry::testing::setup_test_logging();
	let mock = Arc::new(Mock::new());

	let resolver = CachedResolver {
		dns: Arc::new(Resolver::Mock(mock.clone())),
		entries: Arc::new(Mutex::new(HashMap::new())),
	};

	// We should get an error, no IPs yet
	assert_matches!(resolver.resolve("example.com".into()).await, Err(_));
	assert_matches!(resolver.resolve("example.com".into()).await, Err(_));
	// Even once the error is resolved we should not immediately get success
	mock.add_response("example.com", vec![IP3], 60);
	assert_matches!(resolver.resolve("example.com".into()).await, Err(_));
	// But once the retry occurs we will get it
	tokio::time::sleep(ERROR_BACKOFF_BASE + Duration::from_secs(1)).await;
	// Now we should get the IP
	assert_eq!(resolver.resolve("example.com".into()).await.unwrap(), IP3);
}

#[tokio::test]
async fn test_multiple_hostnames() {
	let mock = Arc::new(Mock::new());
	mock.add_response("host1.com", vec![IP1], 60);
	mock.add_response("host2.com", vec![IP2], 60);

	let resolver = CachedResolver {
		dns: Arc::new(Resolver::Mock(mock)),
		entries: Arc::new(Mutex::new(HashMap::new())),
	};

	let ip1 = resolver.resolve("host1.com".into()).await.unwrap();
	let ip2 = resolver.resolve("host2.com".into()).await.unwrap();

	assert_eq!(ip1, Ipv4Addr::new(192, 168, 1, 1));
	assert_eq!(ip2, Ipv4Addr::new(192, 168, 1, 2));
}

#[tokio::test]
async fn test_resolution_failure() {
	let mock = Arc::new(Mock::new());
	// No responses added, so all resolutions will fail

	let resolver = CachedResolver {
		dns: Arc::new(Resolver::Mock(mock)),
		entries: Arc::new(Mutex::new(HashMap::new())),
	};

	let result = resolver.resolve("nonexistent.com".into()).await;
	assert!(result.is_err());
}

#[tokio::test]
async fn test_concurrent_resolution() {
	let mock = Arc::new(Mock::new());
	mock.add_response("example.com", vec![IP1, IP2], 60);

	let resolver = Arc::new(CachedResolver {
		dns: Arc::new(Resolver::Mock(mock)),
		entries: Arc::new(Mutex::new(HashMap::new())),
	});

	// Spawn multiple concurrent resolutions
	let handles: Vec<_> = (0..10)
		.map(|_| {
			let resolver = resolver.clone();
			tokio::spawn(async move { resolver.resolve("example.com".into()).await })
		})
		.collect();

	// All should succeed
	for handle in handles {
		let result = handle.await.unwrap();
		assert!(result.is_ok());
	}
}
