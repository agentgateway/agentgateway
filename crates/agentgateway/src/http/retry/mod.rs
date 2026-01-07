mod body;

use std::num::NonZeroU8;
use std::time::Duration;

pub use body::ReplayBody;

use crate::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[derive(Eq, PartialEq)]
pub struct Policy {
	#[serde(default = "default_attempts")]
	pub attempts: NonZeroU8,
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub backoff: Option<Duration>,
	#[serde(serialize_with = "ser_display_iter", deserialize_with = "de_codes")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<std::num::NonZeroU16>"))]
	pub codes: Box<[http::StatusCode]>,
}

pub fn de_codes<'de: 'a, 'a, D>(deserializer: D) -> Result<Box<[http::StatusCode]>, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = Vec::<u16>::deserialize(deserializer)?;
	let boxed = raw
		.into_iter()
		.map(|c| http::StatusCode::from_u16(c).map_err(serde::de::Error::custom))
		.collect::<Result<Vec<_>, _>>()?;
	Ok(boxed.into_boxed_slice())
}
fn default_attempts() -> NonZeroU8 {
	NonZeroU8::new(1).unwrap()
}
