use arc_swap::{ArcSwap, ArcSwapOption};
use std::sync::Arc;

pub type AtomicOption<T> = Arc<ArcSwapOption<T>>;
pub type Atomic<T> = Arc<ArcSwap<T>>;
