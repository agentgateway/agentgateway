use arc_swap::ArcSwapOption;
use std::sync::Arc;

pub type AtomicOption<T> = Arc<ArcSwapOption<T>>;
