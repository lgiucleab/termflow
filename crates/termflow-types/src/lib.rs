//! Canonical transcript, events, ids, `ErrorClass` and shared DTOs.
//!
//! M0 ships ids, `AppInfo`, `AppEvent` and `ErrorClass`; the transcript (`Turn`, `Part`) lands in M2.

use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

macro_rules! ulid_id {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub ulid::Ulid);

        impl $name {
            pub fn new() -> Self { Self(ulid::Ulid::new()) }
        }
        impl Default for $name {
            fn default() -> Self { Self::new() }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
    )*};
}

ulid_id!(SessionId, TurnId, CallId, AttemptId, BlockId, WorkspaceId);

/// Basic information about the running app, shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// Events published on the core event bus; the UI subscribes to them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    /// Demo heartbeat proving the Rust → UI event path (M0).
    Tick { n: u32 },
}

/// Classified provider error; adapters produce it, only the router acts on it (`BUILD_PROMPT` §7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum ErrorClass {
    TransientRateLimit {
        retry_after: Option<Duration>,
        reset_at: Option<Timestamp>,
    },
    QuotaExhausted {
        reset_at: Option<Timestamp>,
    },
    Overloaded,
    ContextOverflow {
        limit: Option<u32>,
    },
    Auth,
    ContentPolicy,
    BadRequest {
        recoverable_by_stripping_native: bool,
    },
    Cancelled,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_roundtrip() {
        let a = SessionId::new();
        assert_ne!(a, SessionId::new());
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), a);
    }

    #[test]
    fn app_event_wire_shape() {
        let json = serde_json::to_string(&AppEvent::Tick { n: 3 }).unwrap();
        assert_eq!(json, r#"{"type":"tick","n":3}"#);
    }
}
