//! What to do after a failed attempt: the `BUILD_PROMPT` §7.6 action table as a pure function.

use std::time::Duration;

use jiff::Timestamp;
use termflow_types::ErrorClass;

use crate::clock::after;

#[derive(Debug, Clone, PartialEq)]
pub struct RetryPolicy {
    /// Wait for a rate limit only if it lifts within this long; otherwise move on.
    pub transient_max_wait: Duration,
    pub overloaded_retries: u32,
    /// Quota cooldown when the provider gives no reset time.
    pub quota_default: Duration,
    /// Cooldown for a target we moved away from without a known reset.
    pub cooldown: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            transient_max_wait: Duration::from_secs(10),
            overloaded_retries: 2,
            quota_default: Duration::from_secs(3600),
            cooldown: Duration::from_secs(30),
        }
    }
}

/// What to take out of rotation when moving on.
#[derive(Debug, Clone, PartialEq)]
pub enum Cool {
    /// Nothing extra; the health registry counts the failure.
    None,
    /// This (provider, model, credential) until the time.
    Target(Timestamp),
    /// Every model on this credential: until the time, or until the user fixes it (`None`).
    Credential(Option<Timestamp>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Wait, then send to the same target again.
    Retry { after: Duration },
    /// Give up on this target and try the next candidate.
    Next(Cool),
    /// Too big for this target: try `on_context_overflow` / trim the history.
    Overflow,
    /// Refused: try the `on_content_policy` targets (another vendor).
    ContentPolicy,
    /// Strip provider-native parts and retry this target once.
    StripNative,
    /// Show the error; don't fall through the chain.
    Surface,
}

/// `retries`: attempts already retried on this target for this request. `jitter` in `[0, 1)`.
pub fn decide(
    class: &ErrorClass,
    retries: u32,
    stripped: bool,
    p: &RetryPolicy,
    now: Timestamp,
    jitter: f64,
) -> Action {
    match class {
        ErrorClass::TransientRateLimit { retry_after, reset_at } => {
            let wait = retry_after.or_else(|| reset_at.map(|r| crate::clock::until(now, r)));
            match wait {
                Some(w) if w <= p.transient_max_wait && retries < 3 => Action::Retry { after: w },
                Some(w) => Action::Next(Cool::Target(reset_at.unwrap_or_else(|| after(now, w)))),
                None if retries == 0 => Action::Retry {
                    after: backoff(0, jitter),
                },
                None => Action::Next(Cool::Target(after(now, p.cooldown))),
            }
        }
        ErrorClass::QuotaExhausted { reset_at } => Action::Next(Cool::Credential(Some(
            reset_at.unwrap_or_else(|| after(now, p.quota_default)),
        ))),
        ErrorClass::Overloaded if retries < p.overloaded_retries => Action::Retry {
            after: backoff(retries, jitter),
        },
        // Unknown is treated like Overloaded, but retried only once (§7.4).
        ErrorClass::Unknown if retries < 1 => Action::Retry {
            after: backoff(retries, jitter),
        },
        ErrorClass::Overloaded | ErrorClass::Unknown => Action::Next(Cool::None),
        ErrorClass::ContextOverflow { .. } => Action::Overflow,
        ErrorClass::Auth => Action::Next(Cool::Credential(None)),
        ErrorClass::ContentPolicy => Action::ContentPolicy,
        ErrorClass::BadRequest {
            recoverable_by_stripping_native: true,
        } if !stripped => Action::StripNative,
        ErrorClass::BadRequest { .. } | ErrorClass::Cancelled => Action::Surface,
    }
}

/// Exponential backoff with jitter: 1 s, 2 s, 4 s … plus up to half a second.
pub fn backoff(retry: u32, jitter: f64) -> Duration {
    let base = Duration::from_secs(1u64 << retry.min(6));
    base + Duration::from_secs_f64(jitter.clamp(0.0, 1.0) * 0.5)
}

/// Jitter in `[0, 1)` from the ULID generator's randomness (no extra RNG dependency).
pub fn jitter() -> f64 {
    f64::from(u32::try_from(termflow_types::AttemptId::new().0.random() % 1000).unwrap_or(0)) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-05-14T10:00:00Z".parse().unwrap()
    }
    fn d(c: &ErrorClass, retries: u32) -> Action {
        decide(c, retries, false, &RetryPolicy::default(), now(), 0.0)
    }

    #[test]
    fn quota_never_retries_and_cools_the_credential_until_reset() {
        let reset: Timestamp = "2026-06-01T00:00:00Z".parse().unwrap();
        let q = ErrorClass::QuotaExhausted { reset_at: Some(reset) };
        assert_eq!(d(&q, 0), Action::Next(Cool::Credential(Some(reset))));
        let unknown = ErrorClass::QuotaExhausted { reset_at: None };
        assert_eq!(
            d(&unknown, 0),
            Action::Next(Cool::Credential(Some("2026-05-14T11:00:00Z".parse().unwrap())))
        );
    }

    #[test]
    fn overloaded_retries_twice_with_backoff_then_moves_on() {
        let o = ErrorClass::Overloaded;
        assert_eq!(
            d(&o, 0),
            Action::Retry {
                after: Duration::from_secs(1)
            }
        );
        assert_eq!(
            d(&o, 1),
            Action::Retry {
                after: Duration::from_secs(2)
            }
        );
        assert_eq!(d(&o, 2), Action::Next(Cool::None));
        assert_eq!(
            d(&ErrorClass::Unknown, 0),
            Action::Retry {
                after: Duration::from_secs(1)
            }
        );
        assert_eq!(d(&ErrorClass::Unknown, 1), Action::Next(Cool::None));
    }

    #[test]
    fn short_rate_limit_waits_long_one_moves_on() {
        let short = ErrorClass::TransientRateLimit {
            retry_after: Some(Duration::from_secs(3)),
            reset_at: None,
        };
        assert_eq!(
            d(&short, 0),
            Action::Retry {
                after: Duration::from_secs(3)
            }
        );
        let long = ErrorClass::TransientRateLimit {
            retry_after: Some(Duration::from_secs(60)),
            reset_at: None,
        };
        assert_eq!(
            d(&long, 0),
            Action::Next(Cool::Target("2026-05-14T10:01:00Z".parse().unwrap()))
        );
        let bare = ErrorClass::TransientRateLimit {
            retry_after: None,
            reset_at: None,
        };
        assert!(matches!(d(&bare, 0), Action::Retry { .. }));
        assert!(matches!(d(&bare, 1), Action::Next(Cool::Target(_))));
    }

    #[test]
    fn other_rows() {
        assert_eq!(d(&ErrorClass::ContextOverflow { limit: None }, 0), Action::Overflow);
        assert_eq!(d(&ErrorClass::Auth, 0), Action::Next(Cool::Credential(None)));
        assert_eq!(d(&ErrorClass::ContentPolicy, 0), Action::ContentPolicy);
        let sig = ErrorClass::BadRequest {
            recoverable_by_stripping_native: true,
        };
        assert_eq!(d(&sig, 0), Action::StripNative);
        assert_eq!(
            decide(&sig, 0, true, &RetryPolicy::default(), now(), 0.0),
            Action::Surface
        );
        let bad = ErrorClass::BadRequest {
            recoverable_by_stripping_native: false,
        };
        assert_eq!(d(&bad, 0), Action::Surface);
        assert_eq!(d(&ErrorClass::Cancelled, 0), Action::Surface);
    }

    #[test]
    fn backoff_grows_and_jitter_is_bounded() {
        assert_eq!(backoff(2, 0.0), Duration::from_secs(4));
        assert!(backoff(0, 0.999) < Duration::from_millis(1500));
        let j = jitter();
        assert!((0.0..1.0).contains(&j));
    }
}
