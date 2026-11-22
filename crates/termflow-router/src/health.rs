//! Circuit breaker per (provider, model, credential), plus credential-wide blocks for quota and
//! auth failures (`BUILD_PROMPT` §7.5). Pure state; the router persists credential blocks.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;

use crate::clock::after;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetKey {
    pub provider: String,
    pub model: String,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HealthState {
    Closed,
    /// Out of rotation until the time; `None` = until the user fixes the credential.
    Open {
        until: Option<Timestamp>,
        reason: String,
    },
    /// Cooldown over: the next real request is the probe.
    HalfOpen,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialBlock {
    pub until: Option<Timestamp>,
    pub reason: String,
}

#[derive(Default)]
struct Entry {
    failures: Vec<Timestamp>,
    open_until: Option<Timestamp>,
    reason: String,
}

#[derive(Default)]
struct Inner {
    targets: HashMap<TargetKey, Entry>,
    creds: HashMap<String, CredentialBlock>,
}

pub struct HealthRegistry {
    inner: Mutex<Inner>,
    threshold: usize,
    window: Duration,
    cooldown: Duration,
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new(3, Duration::from_secs(60), Duration::from_secs(30))
    }
}

impl HealthRegistry {
    pub fn new(threshold: usize, window: Duration, cooldown: Duration) -> Self {
        Self {
            inner: Mutex::default(),
            threshold: threshold.max(1),
            window,
            // §7.5: at least 30 s.
            cooldown: cooldown.max(Duration::from_secs(30)),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        match self.inner.lock() {
            Ok(mut g) => f(&mut g),
            Err(p) => f(&mut p.into_inner()),
        }
    }

    pub fn state(&self, k: &TargetKey, now: Timestamp) -> HealthState {
        self.with(|i| {
            if let Some(c) = k.credential.as_ref().and_then(|c| i.creds.get(c)) {
                match c.until {
                    Some(u) if u <= now => {}
                    until => {
                        return HealthState::Open {
                            until,
                            reason: c.reason.clone(),
                        };
                    }
                }
            }
            match i
                .targets
                .get(k)
                .and_then(|e| e.open_until.map(|u| (u, e.reason.clone())))
            {
                Some((u, reason)) if u > now => HealthState::Open { until: Some(u), reason },
                Some(_) => HealthState::HalfOpen,
                None => HealthState::Closed,
            }
        })
    }

    pub fn available(&self, k: &TargetKey, now: Timestamp) -> bool {
        !matches!(self.state(k, now), HealthState::Open { .. })
    }

    /// A request succeeded: close the breaker; an expired credential block is dropped too.
    pub fn success(&self, k: &TargetKey, now: Timestamp) {
        self.with(|i| {
            i.targets.remove(k);
            if let Some(c) = &k.credential
                && i.creds.get(c).is_some_and(|b| b.until.is_some_and(|u| u <= now))
            {
                i.creds.remove(c);
            }
        });
    }

    /// Count a failure; opens the breaker after `threshold` failures within `window`.
    /// A failed probe (half-open) re-opens at once.
    pub fn failure(&self, k: &TargetKey, now: Timestamp, reason: &str) {
        let (threshold, window, cooldown) = (self.threshold, self.window, self.cooldown);
        self.with(|i| {
            let e = i.targets.entry(k.clone()).or_default();
            let probing = e.open_until.is_some_and(|u| u <= now);
            e.failures.retain(|t| after(*t, window) > now);
            e.failures.push(now);
            if probing || e.failures.len() >= threshold {
                e.open_until = Some(after(now, cooldown));
                e.reason = reason.into();
                e.failures.clear();
            }
        });
    }

    /// Take one target out of rotation until `until` (rate limit with a known reset).
    pub fn cool_target(&self, k: &TargetKey, until: Timestamp, reason: &str) {
        self.with(|i| {
            let e = i.targets.entry(k.clone()).or_default();
            e.open_until = Some(e.open_until.map_or(until, |u| u.max(until)));
            e.reason = reason.into();
        });
    }

    pub fn block_credential(&self, cred: &str, block: CredentialBlock) {
        self.with(|i| i.creds.insert(cred.into(), block));
    }

    /// The user re-entered the key, or the reset passed.
    pub fn clear_credential(&self, cred: &str) {
        self.with(|i| i.creds.remove(cred));
    }

    pub fn credential_blocks(&self) -> HashMap<String, CredentialBlock> {
        self.with(|i| i.creds.clone())
    }

    /// The earliest time any of `keys` comes back (for auto-resume). `None` if none ever will
    /// on its own (all blocked until the user acts).
    pub fn earliest_reset(&self, keys: &[TargetKey], now: Timestamp) -> Option<Timestamp> {
        keys.iter()
            .filter_map(|k| match self.state(k, now) {
                HealthState::Open { until, .. } => until,
                _ => Some(now),
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(cred: &str) -> TargetKey {
        TargetKey {
            provider: "anthropic".into(),
            model: "m".into(),
            credential: Some(cred.into()),
        }
    }
    fn t(s: &str) -> Timestamp {
        format!("2026-05-14T10:{s}Z").parse().unwrap()
    }

    #[test]
    fn opens_after_threshold_in_window_then_half_opens() {
        let h = HealthRegistry::default();
        h.failure(&k("a"), t("00:00"), "overloaded");
        h.failure(&k("a"), t("00:10"), "overloaded");
        assert_eq!(h.state(&k("a"), t("00:11")), HealthState::Closed);
        h.failure(&k("a"), t("00:20"), "overloaded");
        assert!(!h.available(&k("a"), t("00:21")));
        assert_eq!(h.state(&k("a"), t("00:50")), HealthState::HalfOpen);
        // A failed probe re-opens at once; a successful one closes.
        h.failure(&k("a"), t("00:51"), "overloaded");
        assert!(!h.available(&k("a"), t("00:52")));
        h.success(&k("a"), t("01:30"));
        assert_eq!(h.state(&k("a"), t("01:30")), HealthState::Closed);
    }

    #[test]
    fn failures_outside_the_window_dont_count() {
        let h = HealthRegistry::default();
        h.failure(&k("a"), t("00:00"), "x");
        h.failure(&k("a"), t("00:30"), "x");
        h.failure(&k("a"), t("01:10"), "x");
        assert_eq!(h.state(&k("a"), t("01:11")), HealthState::Closed);
    }

    #[test]
    fn credential_block_covers_every_model_and_expires() {
        let h = HealthRegistry::default();
        h.block_credential(
            "a",
            CredentialBlock {
                until: Some(t("30:00")),
                reason: "quota".into(),
            },
        );
        let other = TargetKey {
            model: "m2".into(),
            ..k("a")
        };
        assert!(!h.available(&other, t("10:00")));
        assert!(h.available(&k("b"), t("10:00")));
        assert!(h.available(&other, t("30:00")));
        assert_eq!(h.earliest_reset(&[k("a")], t("10:00")), Some(t("30:00")));
        h.block_credential(
            "a",
            CredentialBlock {
                until: None,
                reason: "auth".into(),
            },
        );
        assert!(!h.available(&k("a"), t("59:00")));
        assert_eq!(h.earliest_reset(&[k("a")], t("10:00")), None);
        h.clear_credential("a");
        assert!(h.available(&k("a"), t("10:00")));
    }

    #[test]
    fn cool_target_until_reset() {
        let h = HealthRegistry::default();
        h.cool_target(&k("a"), t("05:00"), "rate limit");
        assert!(!h.available(&k("a"), t("04:59")));
        assert_eq!(h.state(&k("a"), t("05:00")), HealthState::HalfOpen);
    }
}
