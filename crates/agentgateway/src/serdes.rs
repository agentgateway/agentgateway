use std::path::PathBuf;

pub use agent_core::serdes::*;
use anyhow::Context;
use openapiv3::OpenAPI;
use serde::de::DeserializeOwned;

use crate::resource_manager::{ResourceFetcher, ResourceKind, ResourceRef};

define_schema_aliases!();

/// Loads [`FileOrInline`] values through the resource manager so file-backed
/// values participate in config reloads.
pub async fn load_file_or_inline(
	value: &FileOrInline,
	resources: &ResourceFetcher,
) -> anyhow::Result<String> {
	match value {
		FileOrInline::Inline(value) => Ok(value.clone()),
		FileOrInline::File { file } => {
			let bytes = resources.fetch(ResourceRef::File(file.clone())).await?;
			String::from_utf8(bytes.to_vec()).context("resource is not valid UTF-8")
		},
	}
}

/// [`load_file_or_inline`] for serde hooks, through the fetcher of the enclosing parse.
pub fn load_file_or_inline_during_parse(value: &FileOrInline) -> anyhow::Result<String> {
	let resources = crate::resource_manager::parse_fetcher();
	futures::executor::block_on(load_file_or_inline(value, &resources))
}

pub fn deser_watched_secret<'de, D>(deserializer: D) -> Result<secrecy::SecretString, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let input = <FileOrInline as serde::Deserialize>::deserialize(deserializer)?;
	load_file_or_inline_during_parse(&input)
		.map(|s| secrecy::SecretString::from(s.trim().to_string()))
		.map_err(serde::de::Error::custom)
}

pub fn deser_watched_secret_option<'de, D>(
	deserializer: D,
) -> Result<Option<secrecy::SecretString>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let Some(input) = <Option<FileOrInline> as serde::Deserialize>::deserialize(deserializer)? else {
		return Ok(None);
	};
	load_file_or_inline_during_parse(&input)
		.map(|s| Some(secrecy::SecretString::from(s.trim().to_string())))
		.map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum FileInlineOrRemote {
	File {
		/// Path to a file on disk to load the value from.
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
	pub async fn load<T: DeserializeOwned>(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<T> {
		let s = self.load_string(resources, kind).await?;
		serde_json::from_str(&s).map_err(Into::into)
	}

	pub async fn load_openapi_schema(&self, resources: &ResourceFetcher) -> anyhow::Result<OpenAPI> {
		let s = self.load_string(resources, ResourceKind::OpenApi).await?;
		stacker::grow(2 * 1024 * 1024, || yaml::from_str::<OpenAPI>(s.as_str()))
	}

	async fn load_string(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<String> {
		Ok(match self {
			FileInlineOrRemote::Inline(s) => s.clone(),
			FileInlineOrRemote::File { .. } | FileInlineOrRemote::Remote { .. } => {
				let bytes = resources
					.fetch(self.as_resource_ref(kind).expect("resource ref"))
					.await?;
				String::from_utf8(bytes.to_vec())?
			},
		})
	}

	fn as_resource_ref(&self, kind: ResourceKind) -> Option<ResourceRef> {
		match self {
			FileInlineOrRemote::File { file } => Some(ResourceRef::File(file.clone())),
			FileInlineOrRemote::Inline(_) => None,
			FileInlineOrRemote::Remote { url } => Some(ResourceRef::Http {
				url: url.clone(),
				kind,
				max_age: None,
			}),
		}
	}
}
