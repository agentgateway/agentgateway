use std::fmt::{Debug, Display};
use std::io;
use std::path::PathBuf;

use anyhow::Context;
#[cfg(feature = "schema")]
pub use schemars::JsonSchema;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serializer};
pub use serde_with;

use crate::client::Client;
use crate::http::Body;

/// Serde yaml represents things different than just as "JSON in YAML format".
/// We don't want this. Instead, we transcode YAML via the JSON module.
pub mod yamlviajson {
	use serde::{de, ser};

	pub fn from_str<T>(s: &str) -> anyhow::Result<T>
	where
		T: for<'de> de::Deserialize<'de>,
	{
		let de_yaml = serde_yaml::Deserializer::from_str(s);
		let mut buf = Vec::with_capacity(128);
		{
			let mut se_json = serde_json::Serializer::new(&mut buf);
			serde_transcode::transcode(de_yaml, &mut se_json)?;
		} // se_json is dropped here, releasing the mutable borrow on buf
		Ok(serde_json_path_to_error::from_slice(&buf)?)
	}

	pub fn to_string<T>(value: &T) -> anyhow::Result<String>
	where
		T: ?Sized + ser::Serialize,
	{
		let js = serde_json::to_string(value)?;
		let mut buf = Vec::with_capacity(128);
		let mut se_yaml = serde_yaml::Serializer::new(&mut buf);
		let de_serde = serde_yaml::Deserializer::from_str(&js);
		serde_transcode::transcode(de_serde, &mut se_yaml)?;
		Ok(String::from_utf8(buf)?)
	}
}

pub use macro_rules_attribute::{apply, attribute_alias};

attribute_alias! {
		#[apply(schema_de!)] = #[serde_with::serde_as] #[derive(Debug, Clone, serde::Deserialize)] #[serde(rename_all = "camelCase", deny_unknown_fields)] #[cfg_attr(feature = "schema", derive(JsonSchema))];
		#[apply(schema_ser!)] = #[serde_with::serde_as] #[derive(Debug, Clone, serde::Serialize)] #[serde(rename_all = "camelCase", deny_unknown_fields)] #[cfg_attr(feature = "schema", derive(JsonSchema))];
		#[apply(schema!)] = #[serde_with::serde_as] #[derive(Debug, Clone, serde::Deserialize, serde::Serialize)] #[serde(rename_all = "camelCase", deny_unknown_fields)] #[cfg_attr(feature = "schema", derive(JsonSchema))];
}

pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
	*t == Default::default()
}

pub mod serde_instant_option {
	use std::time::{Duration, Instant};

	use agent_core::durfmt;
	use agent_core::prelude::AtomicOption;
	use serde::Serializer;

	pub fn serialize<S: Serializer>(
		t: &AtomicOption<Instant>,
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		match t.load().as_ref() {
			None => serializer.serialize_none(),
			Some(t) => {
				let ts: Duration = t
					.checked_duration_since(Instant::now())
					.unwrap_or(Duration::ZERO);
				serializer.serialize_str(durfmt::format(ts).as_str())
			},
		}
	}
}

pub mod serde_dur {
	use std::time::Duration;

	use agent_core::durfmt;
	use serde::{Deserialize, Deserializer, Serializer};

	pub fn serialize<S: Serializer>(t: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&durfmt::format(*t))
	}

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
	where
		D: Deserializer<'de>,
	{
		let input = String::deserialize(deserializer)?;

		durfmt::parse(&input)
			.map_err(|e| serde::de::Error::custom(format!("failed to parse duration: {e:?}")))
	}
}

pub mod serde_dur_option {
	use std::time::Duration;

	use agent_core::durfmt;
	use serde::{Deserialize, Deserializer, Serializer};

	pub fn serialize<S: Serializer>(t: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error> {
		match t {
			None => serializer.serialize_none(),
			Some(t) => serializer.serialize_str(&durfmt::format(*t)),
		}
	}

	pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
	where
		D: Deserializer<'de>,
	{
		let input: Option<String> = Option::deserialize(deserializer)?;

		input
			.as_deref()
			.map(durfmt::parse)
			.transpose()
			.map_err(serde::de::Error::custom)
	}
}

pub fn ser_display_option<S: Serializer, T: Display>(
	t: &Option<T>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	match t {
		None => serializer.serialize_none(),
		Some(t) => serializer.serialize_str(&t.to_string()),
	}
}

pub fn ser_display_iter<S: Serializer, T, TI: Display>(
	t: &T,
	serializer: S,
) -> Result<S::Ok, S::Error>
where
	for<'a> &'a T: IntoIterator<Item = &'a TI>,
{
	let mut seq = serializer.serialize_seq(None)?;
	for el in t {
		seq.serialize_element(&el.to_string())?;
	}
	seq.end()
}

pub fn ser_display<S: Serializer, T: Display>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.serialize_str(&t.to_string())
}

pub fn ser_debug<S: Serializer, T: Debug>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.serialize_str(&format!("{t:?}"))
}

pub fn ser_redact<S: Serializer, T>(_: &T, serializer: S) -> Result<S::Ok, S::Error> {
	serializer.serialize_str("<redacted>")
}

pub fn ser_string_or_bytes<S: Serializer, T: AsRef<[u8]>>(
	t: &T,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	let b = t.as_ref();
	if let Ok(s) = std::str::from_utf8(b) {
		serializer.serialize_str(s)
	} else {
		serializer.serialize_bytes(b)
	}
}

pub fn ser_string_or_bytes_option<S: Serializer, T: AsRef<[u8]>>(
	t: &Option<T>,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	match t {
		None => serializer.serialize_none(),
		Some(t) => ser_string_or_bytes(t, serializer),
	}
}

pub fn ser_bytes<S: Serializer, T: AsRef<[u8]>>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
	let b = t.as_ref();
	if let Ok(s) = std::str::from_utf8(b) {
		serializer.serialize_str(s)
	} else {
		serializer.serialize_str(&hex::encode(b))
	}
}

pub fn de_parse<'de: 'a, 'a, D, T>(deserializer: D) -> Result<T, D::Error>
where
	D: Deserializer<'de>,
	T: TryFrom<&'a str>,
	<T as TryFrom<&'a str>>::Error: Display,
{
	let s: &'a str = <&str>::deserialize(deserializer)?;
	match T::try_from(s) {
		Ok(t) => Ok(t),
		Err(e) => Err(serde::de::Error::custom(e)),
	}
}

pub fn de_parse_option<'de: 'a, 'a, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
	D: Deserializer<'de>,
	T: TryFrom<&'a str>,
	<T as TryFrom<&'a str>>::Error: Display,
{
	let s: Option<&'a str> = Option::deserialize(deserializer)?;
	let Some(s) = s else { return Ok(None) };
	match T::try_from(s) {
		Ok(t) => Ok(Some(t)),
		Err(e) => Err(serde::de::Error::custom(e)),
	}
}

pub fn de_bytes<S: Serializer, T: AsRef<[u8]>>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
	let b = t.as_ref();
	if let Ok(s) = std::str::from_utf8(b) {
		serializer.serialize_str(s)
	} else {
		serializer.serialize_str(&hex::encode(b))
	}
}

pub fn deser_key_from_file<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
	D: Deserializer<'de>,
{
	let input = FileOrInline::deserialize(deserializer)?;

	let k = input
		.load()
		.map_err(|e| serde::de::Error::custom(e.to_string()))?;
	Ok(SecretString::from(k.trim().to_string()))
}

pub fn de_as<'de, I, O, D>(deserializer: D) -> Result<O, D::Error>
where
	D: Deserializer<'de>,
	I: DeserializeOwned,
	O: TryFrom<I>,
	<O as TryFrom<I>>::Error: Display,
{
	let s: I = I::deserialize(deserializer)?;
	O::try_from(s).map_err(serde::de::Error::custom)
}

pub fn de_as_opt<'de, I, O, D>(deserializer: D) -> Result<Option<O>, D::Error>
where
	D: Deserializer<'de>,
	I: DeserializeOwned,
	O: TryFrom<I>,
	<O as TryFrom<I>>::Error: Display,
{
	let s: Option<I> = <Option<I>>::deserialize(deserializer)?;
	match s {
		Some(i) => Ok(Some(O::try_from(i).map_err(serde::de::Error::custom)?)),
		None => Ok(None),
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum FileOrInline {
	File { file: PathBuf },
	Inline(String),
}

impl FileOrInline {
	pub fn load(&self) -> io::Result<String> {
		match self {
			FileOrInline::File { file } => fs_err::read_to_string(file),
			FileOrInline::Inline(s) => Ok(s.clone()),
		}
	}
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum FileInlineOrRemote {
	File {
		file: PathBuf,
	},
	Inline(String),
	Remote {
		#[serde(deserialize_with = "de_parse")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		url: http::Uri,
	},
}

impl FileInlineOrRemote {
	pub async fn load<T: DeserializeOwned>(&self, client: Client) -> anyhow::Result<T> {
		let s = match self {
			FileInlineOrRemote::File { file } => fs_err::tokio::read_to_string(file).await?,
			FileInlineOrRemote::Inline(s) => s.clone(),
			FileInlineOrRemote::Remote { url } => {
				let resp = client
					.simple_call(
						::http::Request::builder()
							.uri(url)
							.body(Body::empty())
							.expect("builder should succeed"),
					)
					.await
					.context(format!("fetch {url}"))?;
				return crate::json::from_response_body::<T>(resp).await;
			},
		};
		serde_json::from_str(&s).map_err(Into::into)
	}
}
