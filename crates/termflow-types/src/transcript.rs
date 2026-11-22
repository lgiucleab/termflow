//! The canonical transcript (`BUILD_PROMPT` §7.1): append-only, provider-neutral, with
//! provider-native reasoning kept verbatim. Adapters render it per request.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{AttemptId, CallId, SessionId, TurnId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Committed,
    /// Being streamed. Never sent to a model; becomes `Discarded` if the app dies mid-stream.
    Partial,
    Discarded,
    Reverted,
    Compacted,
}

/// Which model actually produced an assistant turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ServedBy {
    pub provider: String,
    pub model: String,
    pub credential_id: Option<String>,
    pub attempt_id: AttemptId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultContent {
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    ToolCall {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: CallId,
        content: Vec<ToolResultContent>,
        is_error: bool,
    },
    /// Provider-native reasoning, stored exactly as returned. Re-sent only to the same family
    /// and credential scope; dropped (optionally as summary text) for anyone else.
    Reasoning {
        family: String,
        model: String,
        credential_scope: String,
        native: serde_json::Value,
        summary_text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Turn {
    pub id: TurnId,
    pub session_id: SessionId,
    pub role: Role,
    pub parts: Vec<Part>,
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: Timestamp,
    pub status: TurnStatus,
    pub served_by: Option<ServedBy>,
}

impl Turn {
    pub fn new(session_id: SessionId, role: Role, parts: Vec<Part>, status: TurnStatus) -> Self {
        Self {
            id: TurnId::new(),
            session_id,
            role,
            parts,
            created_at: Timestamp::now(),
            status,
            served_by: None,
        }
    }

    /// Concatenated visible text.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| {
                if let Part::Text { text } = p {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Token usage for one attempt. All counts are what the provider reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_write: u32,
    pub cache_read: u32,
    pub reasoning: u32,
}

impl Usage {
    /// Providers report usage piecemeal (start + end); keep the largest value per field.
    pub fn merge(&mut self, o: Usage) {
        self.input = self.input.max(o.input);
        self.output = self.output.max(o.output);
        self.cache_write = self.cache_write.max(o.cache_write);
        self.cache_read = self.cache_read.max(o.cache_read);
        self.reasoning = self.reasoning.max(o.reasoning);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
    Refusal,
    Other,
}

/// Latest rate-limit headers seen for a target (used by M3's proactive switching).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitSnapshot {
    pub requests_remaining: Option<u64>,
    pub tokens_remaining: Option<u64>,
    pub input_tokens_remaining: Option<u64>,
    pub output_tokens_remaining: Option<u64>,
    pub reset_at: Option<Timestamp>,
    pub retry_after_secs: Option<u64>,
}

/// Provider-neutral stream events (`BUILD_PROMPT` §7.2). Tool-call ids are the provider's.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    /// Readable reasoning (summary) for display.
    ReasoningDelta(String),
    /// A complete provider-native reasoning block, to store verbatim.
    ReasoningDone {
        native: serde_json::Value,
        summary_text: Option<String>,
    },
    ToolCallStart {
        native_id: String,
        name: String,
    },
    ToolCallArgsDelta {
        native_id: String,
        json_fragment: String,
    },
    ToolCallEnd {
        native_id: String,
    },
    Usage(Usage),
    RateLimit(RateLimitSnapshot),
    /// The model that served the request, when the provider says (`OpenRouter`, gateways).
    ServedModel(String),
    Done {
        stop_reason: StopReason,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_json_shape_is_stable() {
        let p = Part::Text { text: "hi".into() };
        assert_eq!(serde_json::to_string(&p).unwrap(), r#"{"type":"text","text":"hi"}"#);
    }

    #[test]
    fn usage_merge_keeps_max() {
        let mut u = Usage {
            input: 10,
            ..Usage::default()
        };
        u.merge(Usage {
            output: 5,
            ..Usage::default()
        });
        assert_eq!(
            u,
            Usage {
                input: 10,
                output: 5,
                ..Usage::default()
            }
        );
    }
}
