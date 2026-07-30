//! Process-wide handle to the model catalog, letting `agent_llm` query model attributes without
//! depending on agentgateway (which owns the catalog). Installed once; tracks hot-reloads on its own.

use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;

/// Read handle to the model catalog; implemented in agentgateway on the cost `ModelCatalog`.
pub trait ModelCatalogHandle: Send + Sync {
	fn model_has_tag(&self, model_id: &str, tag: &str) -> bool;
}

// `Option<Arc<dyn ..>>` is `Sized`, so it fits in an `ArcSwap` (an `ArcSwap<dyn ..>` does not).
type Slot = Option<Arc<dyn ModelCatalogHandle>>;

static MODEL_CATALOG: LazyLock<ArcSwap<Slot>> =
	LazyLock::new(|| ArcSwap::from_pointee(None::<Arc<dyn ModelCatalogHandle>>));

/// Install the catalog handle; call once at startup (it reflects hot-reloads on its own).
pub fn set_model_catalog(handle: Arc<dyn ModelCatalogHandle>) {
	MODEL_CATALOG.store(Arc::new(Some(handle)));
}

/// Whether `model_id` carries `tag`; `false` if no catalog is installed.
pub fn model_has_tag(model_id: &str, tag: &str) -> bool {
	let guard = MODEL_CATALOG.load();
	let slot: &Slot = &guard;
	slot
		.as_ref()
		.is_some_and(|c| c.model_has_tag(model_id, tag))
}

#[cfg(test)]
pub(crate) use test_support::{CATALOG_LOCK, clear, install};

#[cfg(test)]
mod test_support {
	use std::collections::HashSet;

	use super::*;

	/// Serializes tests that install the process-global handle.
	pub(crate) static CATALOG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

	struct TestCatalog {
		tag: &'static str,
		models: HashSet<String>,
	}

	impl ModelCatalogHandle for TestCatalog {
		fn model_has_tag(&self, model_id: &str, tag: &str) -> bool {
			tag == self.tag && self.models.contains(model_id)
		}
	}

	pub(crate) fn install<I: IntoIterator<Item = &'static str>>(tag: &'static str, models: I) {
		set_model_catalog(Arc::new(TestCatalog {
			tag,
			models: models.into_iter().map(str::to_string).collect(),
		}));
	}

	pub(crate) fn clear() {
		MODEL_CATALOG.store(Arc::new(None::<Arc<dyn ModelCatalogHandle>>));
	}
}
