use std::sync::Arc;
use arc_swap::ArcSwapOption;

pub type AtomicOption<T> = Arc<ArcSwapOption<T>>;