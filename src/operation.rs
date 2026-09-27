//! Operation-wide coordination primitives.
//!
//! Kernel module usable by every layer (domain, workflows, infrastructure,
//! inbound), alongside `error`.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::error::{Result, ScanError};

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

    /// Returns `Err(ScanError::Cancelled(context))` when this token has been
    /// cancelled, otherwise `Ok(())`.
    pub fn check(&self, context: &str) -> Result<()> {
        if self.is_cancelled() {
            return Err(ScanError::Cancelled(context.to_string()));
        }
        Ok(())
    }
}

/// Returns `true` when `cancellation` is `Some` and has been cancelled.
///
/// Shared shape for call sites that hold an optional, borrowed token rather
/// than an owned [`CancellationToken`].
pub fn is_cancelled(cancellation: Option<&CancellationToken>) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
}

/// [`CancellationToken::check`] for an optional, borrowed token: `Ok(())`
/// when `cancellation` is `None` or not cancelled, otherwise
/// `Err(ScanError::Cancelled(context))`.
pub fn check_cancellation(cancellation: Option<&CancellationToken>, context: &str) -> Result<()> {
    match cancellation {
        Some(token) => token.check(context),
        None => Ok(()),
    }
}
