//! Ctrl-C routing for CLI operations that own cancellable process trees.

use crate::workflows::operation::CancellationToken;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock, Weak};

type TokenFlag = Weak<AtomicBool>;

static ACTIVE_TOKENS: OnceLock<Mutex<Vec<TokenFlag>>> = OnceLock::new();
static SIGNAL_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

/// Keep a token registered only for the duration of one CLI operation.
struct RegisteredCancellation {
    token: CancellationToken,
    flag: Arc<AtomicBool>,
}

impl RegisteredCancellation {
    fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for RegisteredCancellation {
    fn drop(&mut self) {
        let Some(tokens) = ACTIVE_TOKENS.get() else {
            return;
        };
        let Ok(mut tokens) = tokens.lock() else {
            return;
        };
        tokens.retain(|registered| {
            registered
                .upgrade()
                .is_some_and(|flag| !Arc::ptr_eq(&flag, &self.flag))
        });
    }
}

/// Register a cancellable CLI operation and keep its token alive through `operation`.
pub(super) fn with_registered_token(operation: impl FnOnce(CancellationToken) -> i32) -> i32 {
    let registered = match register() {
        Ok(registered) => registered,
        Err(error) => {
            eprintln!("could not install Ctrl-C handler: {error}");
            return 1;
        }
    };
    let code = operation(registered.token());
    if registered.token().is_cancelled() {
        130
    } else {
        code
    }
}

fn register() -> Result<RegisteredCancellation, String> {
    install_signal_handler()?;
    let token = CancellationToken::new();
    let flag = token.as_arc();
    let tokens = ACTIVE_TOKENS.get_or_init(|| Mutex::new(Vec::new()));
    let mut tokens = tokens
        .lock()
        .map_err(|_| "Ctrl-C token registry is unavailable".to_string())?;
    tokens.retain(|registered| registered.strong_count() > 0);
    tokens.push(Arc::downgrade(&flag));
    Ok(RegisteredCancellation { token, flag })
}

fn install_signal_handler() -> Result<(), String> {
    SIGNAL_HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(cancel_registered_tokens).map_err(|error| format!("{error}"))
        })
        .clone()
}

fn cancel_registered_tokens() {
    let Some(tokens) = ACTIVE_TOKENS.get() else {
        return;
    };
    let Ok(mut tokens) = tokens.lock() else {
        return;
    };
    cancel_tokens(&mut tokens);
}

fn cancel_tokens(tokens: &mut Vec<TokenFlag>) {
    tokens.retain(|registered| match registered.upgrade() {
        Some(flag) => {
            CancellationToken::from_arc(flag).cancel();
            true
        }
        None => false,
    });
}
