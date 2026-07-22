use std::path::PathBuf;

pub use agent_core::serdes::*;
use openapiv3::OpenAPI;
use secrecy::SecretString;
use serde::de::DeserializeOwned;

use crate::resource_manager::{ResourceFetcher, ResourceKind, ResourceRef};

define_schema_aliases!();

pub async fn load_file_or_inline(
	resources: &ResourceFetcher,
	input: &FileOrInline,
) -> anyhow::Result<String> {
	Ok(match input {
		FileOrInline::Inline(s) => s.clone(),
		FileOrInline::File { file } => {
			let bytes = resources.fetch(ResourceRef::File(file.clone())).await?;
			String::from_utf8(bytes.to_vec())?
		},
	})
}

pub async fn load_secret_file_or_inline(
	resources: &ResourceFetcher,
	input: &FileOrInline,
) -> anyhow::Result<SecretString> {
	Ok(SecretString::from(
		load_file_or_inline(resources, input)
			.await?
			.trim()
			.to_string(),
	))
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
		stacker::grow(2 * 1024 * 1024, || {
			yamlviajson::from_str::<OpenAPI>(s.as_str())
		})
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
			}),
		}
	}
}
