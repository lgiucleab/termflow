//! Secrets: the OS keychain (macOS Keychain, Windows Credential Manager, Secret Service) is the
//! only place they live. Config and the database refer to them as `secret://<id>`.

mod redact;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use redact::Redactor;

/// Keychain service name for every `TermFlow` secret.
pub const SERVICE: &str = "com.flowlabs.termflow";

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keychain unavailable or refused access: {0}")]
    Store(String),
}

/// Where secrets are kept. Production uses [`KeychainStore`]; tests use [`MemoryStore`].
pub trait SecretStore: Send + Sync {
    fn set(&self, id: &str, secret: &str) -> Result<(), SecretError>;
    fn get(&self, id: &str) -> Result<Option<String>, SecretError>;
    fn delete(&self, id: &str) -> Result<(), SecretError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct KeychainStore;

fn store_err(e: impl std::fmt::Display) -> SecretError {
    SecretError::Store(e.to_string())
}

impl SecretStore for KeychainStore {
    fn set(&self, id: &str, secret: &str) -> Result<(), SecretError> {
        keyring::Entry::new(SERVICE, id)
            .and_then(|e| e.set_password(secret))
            .map_err(store_err)
    }

    fn get(&self, id: &str) -> Result<Option<String>, SecretError> {
        match keyring::Entry::new(SERVICE, id).and_then(|e| e.get_password()) {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    fn delete(&self, id: &str) -> Result<(), SecretError> {
        match keyring::Entry::new(SERVICE, id).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
    }
}

/// In-memory store for tests (never touches the real keychain).
#[derive(Debug, Default, Clone)]
pub struct MemoryStore(Arc<Mutex<HashMap<String, String>>>);

impl MemoryStore {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SecretStore for MemoryStore {
    fn set(&self, id: &str, secret: &str) -> Result<(), SecretError> {
        self.map().insert(id.into(), secret.into());
        Ok(())
    }
    fn get(&self, id: &str) -> Result<Option<String>, SecretError> {
        Ok(self.map().get(id).cloned())
    }
    fn delete(&self, id: &str) -> Result<(), SecretError> {
        self.map().remove(id);
        Ok(())
    }
}

/// Resolves `secret://<id>` references and registers every value it hands out with the redactor.
#[derive(Clone)]
pub struct Secrets {
    store: Arc<dyn SecretStore>,
    redactor: Redactor,
}

impl Secrets {
    pub fn new(store: Arc<dyn SecretStore>, redactor: Redactor) -> Self {
        Self { store, redactor }
    }

    pub fn set(&self, id: &str, secret: &str) -> Result<(), SecretError> {
        self.redactor.register(secret);
        self.store.set(id, secret)
    }

    pub fn get(&self, id: &str) -> Result<Option<String>, SecretError> {
        let v = self.store.get(id)?;
        if let Some(s) = &v {
            self.redactor.register(s.clone());
        }
        Ok(v)
    }

    pub fn delete(&self, id: &str) -> Result<(), SecretError> {
        self.store.delete(id)
    }

    /// `secret://<id>` → the secret; anything else is not a reference.
    pub fn resolve(&self, reference: &str) -> Result<Option<String>, SecretError> {
        match reference.strip_prefix("secret://") {
            Some(id) => self.get(id),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_registers_for_redaction() {
        let r = Redactor::new();
        let s = Secrets::new(Arc::new(MemoryStore::default()), r.clone());
        s.store.set("k1", "my-very-secret-value").unwrap();
        assert_eq!(
            s.resolve("secret://k1").unwrap().as_deref(),
            Some("my-very-secret-value")
        );
        assert_eq!(s.resolve("plain").unwrap(), None);
        assert_eq!(r.redact("x my-very-secret-value y"), "x [REDACTED] y");
        s.delete("k1").unwrap();
        assert_eq!(s.get("k1").unwrap(), None);
    }
}
