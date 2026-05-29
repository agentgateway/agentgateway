use std::collections::HashMap;

use crate::*;

#[apply(schema_enum!)]
#[derive(Default)]
pub enum Phase {
	#[default]
	Off,
	Request,
	Response,
	Full,
}

impl Phase {
	pub fn runs_request(self) -> bool {
		matches!(self, Phase::Request | Phase::Full)
	}
	pub fn runs_response(self) -> bool {
		matches!(self, Phase::Response | Phase::Full)
	}
}

pub fn resolve(method: &str, methods: &HashMap<String, Phase>) -> Phase {
	methods.get(method).copied().unwrap_or_default()
}
