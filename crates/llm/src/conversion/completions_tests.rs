use super::parse_data_url;

#[test]
fn plain_base64_data_url() {
	assert_eq!(
		parse_data_url("data:application/pdf;base64,JVBERi0xLjQK"),
		Some(("application/pdf", "JVBERi0xLjQK"))
	);
}

#[test]
fn media_type_parameters_are_dropped() {
	// RFC 2397 permits parameters before the encoding marker. Rejecting these used to
	// push callers onto their raw-base64 fallback, sending the header as payload.
	assert_eq!(
		parse_data_url("data:text/plain;charset=utf-8;base64,dGVzdA=="),
		Some(("text/plain", "dGVzdA=="))
	);
}

#[test]
fn empty_media_type_is_preserved() {
	assert_eq!(
		parse_data_url("data:;base64,dGVzdA=="),
		Some(("", "dGVzdA=="))
	);
}

#[test]
fn non_base64_encodings_are_rejected() {
	assert_eq!(parse_data_url("data:image/png,iVBORw0KGgo="), None);
	assert_eq!(parse_data_url("data:text/plain;charset=utf-8,hi"), None);
}

#[test]
fn non_data_urls_are_rejected() {
	assert_eq!(parse_data_url("https://example.com/cat.png"), None);
	assert_eq!(parse_data_url("data:no-comma"), None);
}
