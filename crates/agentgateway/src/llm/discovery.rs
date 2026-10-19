use agent_core::prelude::Strng;

use crate::llm::model_transform::ModelTransformation;
use crate::{apply, cel, schema_enum, schema_ser_schema};

#[apply(schema_enum!)]
#[derive(Default)]
pub enum Discovery {
	/// Expand wildcard model names using the local model catalog.
	#[default]
	Catalog,
	/// List configured model names without expanding wildcards.
	Disabled,
}

/// Links a served model name to its model catalog entries.
#[apply(schema_ser_schema!)]
pub struct ModelDiscovery {
	pub provider: Strng,
	/// Maps catalog model IDs to served model names.
	pub transformation: ModelTransformation,
	/// The upstream model when it is fixed regardless of the served name.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl ModelDiscovery {
	/// Returns None when the upstream model cannot be derived statically from the served name.
	pub fn new(
		provider: Strng,
		model: Option<Strng>,
		transformation: Option<&cel::Expression>,
	) -> Option<Self> {
		let transformation = match transformation {
			Some(expression) => crate::llm::model_transform::reverse_model_transformation(expression)?,
			None => ModelTransformation::Identity,
		};
		Some(Self {
			provider,
			transformation,
			model,
		})
	}

	/// Whether wildcard names can be expanded from the catalog.
	pub fn expands(&self, name: &str) -> bool {
		self.model.is_none() && name.contains('*')
	}

	/// Finds the catalog model ID backing a served name.
	pub fn catalog_model<'a>(&self, served: &str, mut ids: impl Iterator<Item = &'a str>) -> String {
		// A fixed model replaces the served name before the transformation runs.
		let target = self.model.as_deref().unwrap_or(served);
		if !matches!(self.transformation, ModelTransformation::Identity)
			&& let Some(id) = ids.find(|id| {
				self
					.transformation
					.apply(*id)
					.is_some_and(|name| name == target)
			}) {
			return id.to_owned();
		}
		// Otherwise the target is the upstream model: either there is no transformation, or it left
		// the name unchanged (e.g. stripPrefix without the prefix). The catalog resolves provider
		// aliases such as Vertex publisher prefixes.
		target.to_string()
	}
}
