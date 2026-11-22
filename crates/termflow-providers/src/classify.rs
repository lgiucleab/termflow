//! Error classification (`BUILD_PROMPT` §7.4): status code **plus** body error codes. A 429 can be
//! "wait 3 s" or "quota gone until next month"; only the body tells which.
// shortcut: patterns live in code; M3 adds the overridable `error_patterns.toml` data file.

use jiff::{Timestamp, ToSpan, civil::Date};
use reqwest::header::HeaderMap;
use serde_json::Value;
use termflow_types::{ErrorClass, WireFormat};

const OPENAI_QUOTA_CODES: &[&str] = &[
    "insufficient_quota",
    "credit_balance_exhausted",
    "organization_spend_limit_exceeded",
    "project_spend_limit_exceeded",
    "organization_usage_limit_exceeded",
];

const CONTEXT_PATTERNS: &[&str] = &[
    "prompt is too long",
    "context length",
    "context_length_exceeded",
    "maximum context",
    "context window",
    "too many tokens",
    "request_too_large",
];

const NATIVE_PATTERNS: &[&str] = &["thinking", "signature", "reasoning", "encrypted_content"];

pub fn retry_after_secs(h: &HeaderMap) -> Option<u64> {
    h.get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

/// 00:00 UTC on the 1st of next month: when an Anthropic spend limit resets.
fn first_of_next_month(now: Timestamp) -> Option<Timestamp> {
    let d = now.to_zoned_datetime(jiff::tz::TimeZone::UTC).date();
    let next: Date = d.first_of_month().checked_add(1.month()).ok()?;
    next.to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

fn s<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |acc, k| acc.get(*k)).and_then(Value::as_str)
}

/// Classify a non-2xx HTTP response, or an error object that arrived mid-stream (`status: None`).
pub fn classify_http(wire: WireFormat, status: Option<u16>, headers: &HeaderMap, body: &Value) -> ErrorClass {
    let retry_after = retry_after_secs(headers);
    let msg = s(body, &["error", "message"])
        .or(s(body, &["message"]))
        .unwrap_or_default()
        .to_lowercase();
    let etype = s(body, &["error", "type"]).or(s(body, &["type"])).unwrap_or_default();
    let code = s(body, &["error", "code"])
        .or(s(body, &["code"]))
        .or(s(body, &["error", "details", "error_code"]))
        .unwrap_or_default();
    let detail_code = s(body, &["error", "details", "error_code"]).unwrap_or_default();
    // OpenRouter puts the HTTP status in error.code as a number, and a typed code in metadata.
    let num_code = body.get("error").and_then(|e| e.get("code")).and_then(Value::as_u64);
    let or_type = s(body, &["error", "metadata", "error_type"]).unwrap_or_default();
    let status = status.or_else(|| num_code.and_then(|c| u16::try_from(c).ok()));

    // Quota / spend caps: never retry the same target.
    if detail_code == "enforced_spend_limit_reached" && retry_after.is_none() {
        return ErrorClass::QuotaExhausted {
            reset_at: first_of_next_month(Timestamp::now()),
        };
    }
    if OPENAI_QUOTA_CODES.contains(&code) || etype == "insufficient_quota" || etype == "billing_error" {
        return ErrorClass::QuotaExhausted { reset_at: None };
    }
    if msg.contains("reached your specified api usage limits") {
        return ErrorClass::QuotaExhausted { reset_at: None };
    }
    if or_type == "content_policy_violation" || code == "content_policy_violation" || code == "content_filter" {
        return ErrorClass::ContentPolicy;
    }
    // Mid-stream error objects carry only a type/code.
    match etype {
        "overloaded_error" | "api_error" | "server_error" | "service_unavailable_error" => {
            return ErrorClass::Overloaded;
        }
        // (the spend-cap variant of rate_limit_error returned above)
        "rate_limit_error" => {
            return ErrorClass::TransientRateLimit {
                retry_after: retry_after.map(std::time::Duration::from_secs),
                reset_at: None,
            };
        }
        "authentication_error" | "permission_error" => return ErrorClass::Auth,
        _ => {}
    }
    if matches!(code, "server_error" | "server_is_overloaded") {
        return ErrorClass::Overloaded;
    }
    if matches!(code, "rate_limit_exceeded" | "slow_down") {
        return ErrorClass::TransientRateLimit {
            retry_after: retry_after.map(std::time::Duration::from_secs),
            reset_at: None,
        };
    }
    let _ = wire; // per-wire overrides land here as patterns accumulate
    match status {
        Some(401 | 403) => ErrorClass::Auth,
        Some(402) => ErrorClass::QuotaExhausted { reset_at: None },
        Some(413) => ErrorClass::ContextOverflow { limit: None },
        Some(429) => ErrorClass::TransientRateLimit {
            retry_after: retry_after.map(std::time::Duration::from_secs),
            reset_at: None,
        },
        Some(408 | 500 | 502 | 503 | 504 | 529) => ErrorClass::Overloaded,
        Some(400 | 422) => {
            if CONTEXT_PATTERNS.iter().any(|p| msg.contains(p)) || code == "context_length_exceeded" {
                ErrorClass::ContextOverflow { limit: None }
            } else {
                ErrorClass::BadRequest {
                    recoverable_by_stripping_native: NATIVE_PATTERNS.iter().any(|p| msg.contains(p)),
                }
            }
        }
        _ => ErrorClass::Unknown,
    }
}

/// Human-readable message from any of the providers' error bodies.
pub fn error_message(body: &Value, fallback: &str) -> String {
    s(body, &["error", "message"])
        .or(s(body, &["message"]))
        .or(body.get("error").and_then(Value::as_str))
        .map_or_else(|| fallback.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(retry: Option<&str>) -> HeaderMap {
        let mut m = HeaderMap::new();
        if let Some(r) = retry {
            m.insert("retry-after", r.parse().unwrap());
        }
        m
    }

    const A: WireFormat = WireFormat::AnthropicMessages;
    const O: WireFormat = WireFormat::OpenaiResponses;

    #[test]
    fn anthropic_spend_limit_without_retry_after_is_quota_until_next_month() {
        let body = json!({"type":"error","error":{"type":"rate_limit_error","message":"limit","details":{"error_code":"enforced_spend_limit_reached"}}});
        let ErrorClass::QuotaExhausted { reset_at: Some(at) } = classify_http(A, Some(429), &h(None), &body) else {
            panic!()
        };
        let z = at.to_zoned_datetime(jiff::tz::TimeZone::UTC);
        assert_eq!((z.day(), z.hour(), z.minute()), (1, 0, 0));
        assert!(at > Timestamp::now());
    }

    #[test]
    fn plain_429_with_retry_after_is_transient() {
        let body = json!({"type":"error","error":{"type":"rate_limit_error","message":"slow down"}});
        assert_eq!(
            classify_http(A, Some(429), &h(Some("3")), &body),
            ErrorClass::TransientRateLimit {
                retry_after: Some(std::time::Duration::from_secs(3)),
                reset_at: None
            }
        );
    }

    #[test]
    fn anthropic_400_usage_limit_is_quota() {
        let body = json!({"type":"error","error":{"type":"invalid_request_error","message":"You have reached your specified API usage limits."}});
        assert!(matches!(
            classify_http(A, Some(400), &h(None), &body),
            ErrorClass::QuotaExhausted { .. }
        ));
    }

    #[test]
    fn openai_quota_codes() {
        for code in OPENAI_QUOTA_CODES {
            let body = json!({"error":{"type":"insufficient_quota","code":code,"message":"x"}});
            assert!(
                matches!(
                    classify_http(O, Some(429), &h(None), &body),
                    ErrorClass::QuotaExhausted { .. }
                ),
                "{code}"
            );
        }
    }

    #[test]
    fn overloaded_family() {
        let e = json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}});
        assert_eq!(classify_http(A, Some(529), &h(None), &e), ErrorClass::Overloaded);
        let o = json!({"error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"x"}});
        assert_eq!(classify_http(O, Some(503), &h(None), &o), ErrorClass::Overloaded);
        assert_eq!(
            classify_http(O, Some(502), &h(None), &json!({})),
            ErrorClass::Overloaded
        );
    }

    #[test]
    fn context_overflow_413_and_400_message() {
        assert!(matches!(
            classify_http(A, Some(413), &h(None), &json!({})),
            ErrorClass::ContextOverflow { .. }
        ));
        let b = json!({"error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}});
        assert!(matches!(
            classify_http(A, Some(400), &h(None), &b),
            ErrorClass::ContextOverflow { .. }
        ));
    }

    #[test]
    fn auth_and_bad_request_variants() {
        let a = json!({"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}});
        assert_eq!(classify_http(A, Some(401), &h(None), &a), ErrorClass::Auth);
        assert_eq!(classify_http(A, Some(403), &h(None), &json!({})), ErrorClass::Auth);
        let sig = json!({"error":{"type":"invalid_request_error","message":"Invalid signature in thinking block"}});
        assert_eq!(
            classify_http(A, Some(400), &h(None), &sig),
            ErrorClass::BadRequest {
                recoverable_by_stripping_native: true
            }
        );
        let other = json!({"error":{"type":"invalid_request_error","message":"max_tokens: must be positive"}});
        assert_eq!(
            classify_http(A, Some(400), &h(None), &other),
            ErrorClass::BadRequest {
                recoverable_by_stripping_native: false
            }
        );
    }

    #[test]
    fn openrouter_mid_stream_error_and_402() {
        let e =
            json!({"error":{"code":502,"message":"provider down","metadata":{"error_type":"provider_unavailable"}}});
        assert_eq!(
            classify_http(WireFormat::OpenaiChat, None, &h(None), &e),
            ErrorClass::Overloaded
        );
        let q = json!({"error":{"code":402,"message":"Insufficient credits"}});
        assert!(matches!(
            classify_http(WireFormat::OpenaiChat, None, &h(None), &q),
            ErrorClass::QuotaExhausted { .. }
        ));
        let p = json!({"error":{"code":403,"message":"flagged","metadata":{"error_type":"content_policy_violation"}}});
        assert_eq!(
            classify_http(WireFormat::OpenaiChat, None, &h(None), &p),
            ErrorClass::ContentPolicy
        );
    }

    #[test]
    fn unknown_never_panics() {
        assert_eq!(
            classify_http(A, Some(418), &h(None), &json!("teapot")),
            ErrorClass::Unknown
        );
        assert_eq!(classify_http(A, None, &h(None), &json!(null)), ErrorClass::Unknown);
    }
}
