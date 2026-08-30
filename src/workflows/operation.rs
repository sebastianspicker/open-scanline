//! Operation-wide coordination primitives.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// A cloneable cancellation signal shared by workflows and opened sessions.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_arc(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn as_arc(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }
}
