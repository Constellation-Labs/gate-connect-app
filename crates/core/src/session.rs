//! Session-scoped credential cache.
//!
//! Stores the auth_token from the current user session so audit events can be
//! emitted without re-prompting for keychain access. The cache is populated
//! when the user enables the proxy (which already requires account load), and
//! retrieved by disable, config changes, and other audit call sites.
//!
//! Cleared on app exit (session-scoped, not persisted to disk).

use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone)]
pub struct SessionContext {
    auth_token: Arc<Mutex<Option<String>>>,
}

impl SessionContext {
    fn new() -> Self {
        SessionContext {
            auth_token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_token(&self, token: String) {
        if let Ok(mut guard) = self.auth_token.lock() {
            *guard = Some(token);
        }
    }

    pub fn get_token(&self) -> Option<String> {
        self.auth_token.lock().ok().and_then(|guard| guard.clone())
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.auth_token.lock() {
            *guard = None;
        }
    }
}

static SESSION: OnceLock<SessionContext> = OnceLock::new();

fn session() -> &'static SessionContext {
    SESSION.get_or_init(SessionContext::new)
}

pub fn set_auth_token(token: String) {
    session().set_token(token);
}

pub fn get_auth_token() -> Option<String> {
    session().get_token()
}

pub fn clear_auth_token() {
    session().clear();
}
