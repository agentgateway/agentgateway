use std::collections::HashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Phase {
	#[default]
	Off,
	Request,
	Response,
	Both,
}

impl Phase {
	pub fn runs_request(self) -> bool {
		matches!(self, Phase::Request | Phase::Both)
	}
	pub fn runs_response(self) -> bool {
		matches!(self, Phase::Response | Phase::Both)
	}
}

pub fn resolve(method: &str, methods: &HashMap<String, Phase>) -> Phase {
	methods.get(method).copied().unwrap_or_default()
}
