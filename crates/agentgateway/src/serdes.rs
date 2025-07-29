use std::fmt::{Debug, Display};
use std::path::PathBuf;
use std::{fs, io};

use anyhow::Context;
#[cfg(feature = "schema")]
pub use schemars::JsonSchema;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
pub use serde_with;

use crate::client::Client;
use crate::http::Body;

/// Serde yaml represents things different than just as "JSON in YAML format".
/// We don't want this. Instead, we transcode YAML via the JSON module.
pub mod yamlviajson {
	use futures_util::AsyncReadExt;
	use serde::{Deserialize, de, ser};
	use serde_yaml::to_writer;

	pub fn from_str<T>(s: &str) -> anyhow::Result<T>
	where
		T: for<'de> de::Deserialize<'de>,
	{
		let mut de_yaml = serde_yaml::Deserializer::from_str(s);
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
		let mut de_serde = serde_yaml::Deserializer::from_str(&js);
		serde_transcode::transcode(de_serde, &mut se_yaml)?;
		Ok(String::from_utf8(buf)?)
	}
}

pub fn is_default<T: Default + PartialEq>(t: &T) -> bool {
	*t == Default::default()
}

pub mod serde_dur {
	use std::fmt::Display;

	use duration_str::HumanFormat;
	pub use duration_str::deserialize_duration as deserialize;
	use serde::Serializer;

	pub fn serialize<S: Serializer, T: HumanFormat>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(&t.human_format())
	}
}

pub mod serde_dur_option {
	use std::fmt::Display;

	use duration_str::HumanFormat;
	pub use duration_str::deserialize_option_duration as deserialize;
	use serde::Serializer;

	pub fn serialize<S: Serializer, T: HumanFormat>(
		t: &Option<T>,
		serializer: S,
	) -> Result<S::Ok, S::Error> {
		match t {
			None => serializer.serialize_none(),
			Some(t) => serializer.serialize_str(&t.human_format()),
		}
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

pub fn ser_redact<S: Serializer, T>(t: &T, serializer: S) -> Result<S::Ok, S::Error> {
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
				return crate::json::from_body::<T>(resp.into_body()).await;
			},
		};
		serde_json::from_str(&s).map_err(Into::into)
	}
}
