use std::sync::Arc;

use arc_swap::{ArcSwap, ArcSwapOption};

pub type AtomicOption<T> = Arc<ArcSwapOption<T>>;
pub type Atomic<T> = Arc<ArcSwap<T>>;
