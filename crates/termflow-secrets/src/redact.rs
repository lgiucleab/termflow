//! Redaction of known secrets and common token patterns in logs and model-bound text.

use std::sync::{Arc, LazyLock, RwLock};

use regex::Regex;

const MASK: &str = "[REDACTED]";

#[allow(clippy::expect_used)] // literal patterns, checked by tests
static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"sk-[A-Za-z0-9_\-]{8,}",                                   // OpenAI / Anthropic style keys
        r"gh[pousr]_[A-Za-z0-9]{20,}",                              // GitHub tokens
        r"(?:AKIA|ASIA)[0-9A-Z]{16}",                               // AWS access key ids
        r"eyJ[A-Za-z0-9_\-]+\.eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+", // JWTs
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid redaction regex"))
    .collect()
});

/// Shared set of known secret values plus built-in token patterns.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    known: Arc<RwLock<Vec<String>>>,
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a concrete secret value (e.g. a key just read from the keychain).
    pub fn register(&self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.len() < 4 {
            return; // masking tiny strings would mangle ordinary text
        }
        if let Ok(mut known) = self.known.write() {
            known.push(secret);
        }
    }

    /// Mask known secrets and common token patterns in `text`.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Ok(known) = self.known.read() {
            for s in known.iter() {
                out = out.replace(s.as_str(), MASK);
            }
        }
        for re in PATTERNS.iter() {
            out = re.replace_all(&out, MASK).into_owned();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_patterns_and_known_values() {
        let r = Redactor::new();
        r.register("hunter2-secret");
        let s = r.redact(
            "key=sk-ant-abcdefghijk gh=ghp_abcdefghijklmnopqrstuv aws=AKIAABCDEFGHIJKLMNOP \
             jwt=eyJhbGc.eyJzdWIi.sig_1 pw=hunter2-secret ok=plain",
        );
        assert!(!s.contains("sk-ant"));
        assert!(!s.contains("ghp_"));
        assert!(!s.contains("AKIA"));
        assert!(!s.contains("eyJ"));
        assert!(!s.contains("hunter2"));
        assert!(s.contains("ok=plain"));
    }
}
