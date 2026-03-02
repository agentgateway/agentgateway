use ::http::{HeaderMap, HeaderName, HeaderValue};

use super::remoteratelimit::proto;

/// Convert proto `HeaderValue` entries into an HTTP `HeaderMap`.
///
/// Malformed entries (invalid header name or value) are silently skipped.
pub(crate) fn process_proto_headers(hm: &mut HeaderMap, headers: Vec<proto::HeaderValue>) {
	for h in headers {
		let Ok(hn) = HeaderName::from_bytes(h.key.as_bytes()) else {
			continue;
		};
		let hv = if !h.value.is_empty() {
			HeaderValue::from_bytes(h.value.as_bytes())
		} else if !h.raw_value.is_empty() {
			HeaderValue::from_bytes(&h.raw_value)
		} else {
			continue;
		};
		let Ok(hv) = hv else {
			continue;
		};
		hm.insert(hn, hv);
	}
}
