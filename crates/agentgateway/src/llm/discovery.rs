use agent_core::prelude::Strng;

use crate::llm::model_transform::ModelTransformation;
use crate::{apply, schema_enum, schema_ser_schema};

#[apply(schema_enum!)]
#[derive(Default)]
pub enum Discovery {
	/// Expand wildcard model names using the local model catalog.
	#[default]
	Catalog,
	/// List configured model names without expanding wildcards.
	Disabled,
}

#[apply(schema_ser_schema!)]
pub struct ModelDiscovery {
	pub provider: Strng,
	pub transformation: ModelTransformation,
}
