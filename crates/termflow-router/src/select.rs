//! Chain entries (`provider/model@credential-label`) → concrete candidates, and the capability
//! checks of `BUILD_PROMPT` §7.6 step 2.

use termflow_catalog::ModelCaps;
use termflow_config::Require;
use termflow_types::Credential;

use crate::health::TargetKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub provider: String,
    pub model: String,
    pub label: Option<String>,
}

/// `anthropic/model@work`, `openrouter/vendor/model@key`, `ollama/local`. The model part may
/// contain `/`; the label is after the last `@`.
pub fn parse_entry(s: &str) -> Option<Entry> {
    let s = s.trim();
    let (pm, label) = match s.rsplit_once('@') {
        Some((pm, l)) if !l.is_empty() => (pm, Some(l.to_owned())),
        _ => (s, None),
    };
    let (provider, model) = pm.split_once('/')?;
    (!provider.is_empty() && !model.is_empty()).then(|| Entry {
        provider: provider.to_owned(),
        model: model.to_owned(),
        label,
    })
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub cred: Credential,
    pub model: String,
    pub key: TargetKey,
}

impl Candidate {
    pub fn new(cred: Credential, model: &str) -> Self {
        let key = TargetKey {
            provider: cred.kind.id().into(),
            model: model.into(),
            credential: Some(cred.id.clone()),
        };
        Self {
            cred,
            model: model.into(),
            key,
        }
    }

    /// `provider/model@label`, for events and the paused panel.
    pub fn label(&self) -> String {
        format!("{}/{}@{}", self.key.provider, self.model, self.cred.label)
    }
}

/// The credential an entry names: same provider, and the label (or id) when given.
pub fn resolve(e: &Entry, creds: &[Credential]) -> Option<Candidate> {
    creds
        .iter()
        .find(|c| c.kind.id() == e.provider && e.label.as_ref().is_none_or(|l| c.label == *l || c.id == *l))
        .map(|c| Candidate::new(c.clone(), &e.model))
}

/// Resolve a list of entries, skipping ones that name no configured credential.
pub fn resolve_all(entries: &[String], creds: &[Credential]) -> Vec<Candidate> {
    entries
        .iter()
        .filter_map(|s| {
            let c = parse_entry(s).and_then(|e| resolve(&e, creds));
            if c.is_none() {
                tracing::warn!(entry = %s, "profile entry names no configured credential; skipped");
            }
            c
        })
        .collect()
}

/// Unknown capabilities never disqualify; a known "no" does.
pub fn capable(caps: Option<&ModelCaps>, req: &Require) -> bool {
    let Some(c) = caps else { return true };
    !(req.tools && c.tools == Some(false)
        || req.vision && c.vision == Some(false)
        || req.reasoning && c.reasoning == Some(false))
}

/// Whether a prompt of `est` tokens plus `max_out` output fits the model's context window.
pub fn fits(caps: Option<&ModelCaps>, est: u32, max_out: u32) -> bool {
    caps.and_then(|c| c.context)
        .is_none_or(|ctx| u64::from(est) + u64::from(max_out.min(ctx / 4)) <= u64::from(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use termflow_types::{ProviderKind, WireFormat};

    fn cred(id: &str, kind: ProviderKind, label: &str) -> Credential {
        Credential {
            id: id.into(),
            kind,
            label: label.into(),
            base_url: String::new(),
            wire: WireFormat::OpenaiChat,
            has_secret: false,
            created_at: jiff::Timestamp::now(),
        }
    }

    #[test]
    fn parses_entries() {
        assert_eq!(
            parse_entry("openrouter/vendor/model@key"),
            Some(Entry {
                provider: "openrouter".into(),
                model: "vendor/model".into(),
                label: Some("key".into())
            })
        );
        assert_eq!(parse_entry("ollama/local").unwrap().label, None);
        assert!(parse_entry("nomodel").is_none());
        assert!(parse_entry("/x").is_none());
    }

    #[test]
    fn resolves_by_provider_and_label_or_id() {
        let creds = [
            cred("c1", ProviderKind::Anthropic, "personal"),
            cred("c2", ProviderKind::Anthropic, "work"),
            cred("c3", ProviderKind::Ollama, "local"),
        ];
        let r = resolve_all(
            &[
                "anthropic/m@work".into(),
                "anthropic/m@c1".into(),
                "ollama/x".into(),
                "openai/m@nope".into(),
            ],
            &creds,
        );
        let ids: Vec<_> = r.iter().map(|c| c.cred.id.as_str()).collect();
        assert_eq!(ids, ["c2", "c1", "c3"]);
        assert_eq!(r[0].label(), "anthropic/m@work");
    }

    #[test]
    fn capability_and_fit() {
        let caps = ModelCaps {
            context: Some(8000),
            tools: Some(false),
            ..Default::default()
        };
        assert!(!capable(Some(&caps), &Require::default()));
        assert!(capable(None, &Require::default()));
        assert!(fits(Some(&caps), 5000, 32_000)); // output reservation is capped at ctx/4
        assert!(!fits(Some(&caps), 7000, 32_000));
        assert!(fits(None, u32::MAX, 0));
    }
}
