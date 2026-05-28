use itertools::Itertools;

use super::proto;
use crate::http::ext_proc::proto::HeaderMutation;
use crate::http::{self, envoy_proto_common};

pub(super) fn apply_header_mutations_request(req: &mut http::Request, h: Option<&HeaderMutation>) {
	if let Some(hm) = h {
		for rm in &hm.remove_headers {
			req.headers_mut().remove(rm);
		}
		for set in &hm.set_headers {
			envoy_proto_common::apply_header_option(&mut req.into(), set);
		}
	}
}

pub(super) fn apply_header_mutations_response(
	resp: &mut http::Response,
	h: Option<&HeaderMutation>,
) {
	if let Some(hm) = h {
		for rm in &hm.remove_headers {
			resp.headers_mut().remove(rm);
		}
		for set in &hm.set_headers {
			envoy_proto_common::apply_header_option(&mut resp.into(), set);
		}
	}
}

pub(super) fn req_to_header_map(req: &http::Request) -> Option<proto::HeaderMap> {
	let mut pseudo = crate::http::get_request_pseudo_headers(req);
	let has_scheme = pseudo
		.iter()
		.any(|(p, _)| matches!(p, crate::http::HeaderOrPseudo::Scheme));
	if !has_scheme {
		// Default to http when scheme is not explicitly present on the request URI
		pseudo.push((crate::http::HeaderOrPseudo::Scheme, "http".to_string()));
	}
	let pseudo_header_pairs: Vec<(String, String)> = pseudo
		.into_iter()
		.map(|(p, v)| (p.to_string(), v))
		.collect();
	to_header_map_extra(
		req.headers(),
		&pseudo_header_pairs
			.iter()
			.map(|(k, v)| (k.as_str(), v.as_str()))
			.collect::<Vec<_>>(),
	)
}

pub(super) fn resp_to_header_map(res: &http::Response) -> Option<proto::HeaderMap> {
	to_header_map_extra(res.headers(), &[(":status", res.status().as_str())])
}

pub(super) fn to_header_map(headers: &http::HeaderMap) -> Option<proto::HeaderMap> {
	to_header_map_extra(headers, &[])
}

fn to_header_map_extra(
	headers: &http::HeaderMap,
	additional_headers: &[(&str, &str)],
) -> Option<proto::HeaderMap> {
	let h = headers
		.iter()
		.map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			value: String::new(),
			raw_value: v.as_bytes().to_vec(),
		})
		.chain(additional_headers.iter().map(|(k, v)| proto::HeaderValue {
			key: k.to_string(),
			value: v.to_string(),
			raw_value: vec![],
		}))
		.collect_vec();
	Some(proto::HeaderMap { headers: h })
}
