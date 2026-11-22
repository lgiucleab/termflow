//! Canonical transcript, events, ids, `ErrorClass` and shared DTOs.
//!
//! M0 ships ids, `AppInfo`, `AppEvent` and `ErrorClass`; the transcript (`Turn`, `Part`) lands in M2.

mod transcript;

use std::time::Duration;

pub use transcript::*;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

macro_rules! ulid_id {
    ($($name:ident),*) => {$(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[cfg_attr(feature = "specta", derive(specta::Type))]
        #[serde(transparent)]
        pub struct $name(#[cfg_attr(feature = "specta", specta(type = String))] pub ulid::Ulid);

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

ulid_id!(SessionId, TurnId, CallId, AttemptId, BlockId, WorkspaceId, TerminalId);

/// Basic information about the running app, shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// Events published on the core event bus; the UI subscribes to them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    /// A command started running in a terminal (shell integration mark `C`).
    BlockStarted { block: Block },
    /// A command finished (mark `D`, or the next prompt if `D` was lost).
    BlockFinished { block: Block },
    /// Shell marks arrived out of order or broken; blocks are off for this terminal.
    BlocksDegraded { terminal_id: TerminalId, reason: String },
    /// The terminal's process exited.
    TerminalExited {
        terminal_id: TerminalId,
        exit_code: Option<u32>,
    },
    /// An assistant turn started streaming.
    AgentTurnStarted {
        session_id: SessionId,
        turn_id: TurnId,
        model: String,
    },
    /// Streamed text or readable reasoning for the turn in progress.
    AgentDelta {
        session_id: SessionId,
        turn_id: TurnId,
        text: String,
        reasoning: bool,
    },
    /// A turn was committed to the transcript (persisted before this event is sent).
    AgentTurnCommitted { turn: Turn },
    /// The request failed; the partial turn was discarded.
    AgentError {
        session_id: SessionId,
        turn_id: Option<TurnId>,
        class: String,
        message: String,
    },
    /// A tool call is waiting for the user's decision.
    ToolApprovalNeeded {
        session_id: SessionId,
        call_id: CallId,
        tool: String,
        summary: String,
        /// Diff or command shown on the approval card.
        preview: Option<String>,
    },
    /// A tool call started executing.
    ToolRunning { session_id: SessionId, call_id: CallId },
    /// The agent loop finished (final answer, error, cancel, or step limit).
    AgentIdle { session_id: SessionId },
}

/// Request/response format a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum WireFormat {
    AnthropicMessages,
    OpenaiResponses,
    OpenaiChat,
    Ollama,
}

/// Provider presets offered in settings; `Custom` takes any base URL and wire format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
    Custom,
}

impl ProviderKind {
    /// Default base URL and wire format for a preset.
    pub fn defaults(self) -> Option<(&'static str, WireFormat)> {
        match self {
            Self::Anthropic => Some(("https://api.anthropic.com", WireFormat::AnthropicMessages)),
            Self::Openai => Some(("https://api.openai.com/v1", WireFormat::OpenaiResponses)),
            Self::Openrouter => Some(("https://openrouter.ai/api/v1", WireFormat::OpenaiChat)),
            Self::Ollama => Some(("http://localhost:11434", WireFormat::Ollama)),
            Self::Custom => None,
        }
    }

    /// Provider id used in model refs and `served_by`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Openrouter => "openrouter",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }
}

/// A configured provider connection. The API key itself lives only in the OS keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Credential {
    pub id: String,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub wire: WireFormat,
    pub has_secret: bool,
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub created_at: Timestamp,
}

/// A model offered by a provider's models endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u32>,
}

/// A project folder the user opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Workspace {
    pub id: String,
    pub path: String,
    pub name: String,
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub last_opened: Option<Timestamp>,
}

/// An agent chat session as listed in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AgentSession {
    pub id: SessionId,
    pub workspace_id: Option<String>,
    /// `build` or `plan`.
    pub mode: String,
    pub title: Option<String>,
    pub credential_id: Option<String>,
    pub model: Option<String>,
    pub reasoning: bool,
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub updated_at: Timestamp,
}

/// Which family a shell belongs to; decides how shell integration is injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ShellKind {
    Zsh,
    Bash,
    Fish,
    Nu,
    Pwsh,
    PowerShell,
    Cmd,
    Wsl,
    Other,
}

/// A shell the user can open a terminal with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct ShellSpec {
    /// Stable id for settings and the UI, e.g. `zsh`, `pwsh`, `wsl:Ubuntu`.
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub kind: ShellKind,
}

/// An agent CLI found on PATH (Claude Code, Codex, ...). Names are plain text, no logos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AgentCli {
    pub id: String,
    pub name: String,
    pub program: String,
}

/// One command run in a terminal, as delimited by shell integration marks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Block {
    pub id: BlockId,
    pub terminal_id: TerminalId,
    /// 0-based index of this command within its terminal; the UI matches its own marks by it.
    pub seq: u32,
    pub command: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub shell: String,
    pub exit_code: Option<i32>,
    /// RFC 3339.
    #[cfg_attr(feature = "specta", specta(type = String))]
    pub started_at: Timestamp,
    #[cfg_attr(feature = "specta", specta(type = Option<String>))]
    pub finished_at: Option<Timestamp>,
    /// The chat whose shell ran this command (unified timeline, decision 0002).
    #[serde(default)]
    pub chat_session_id: Option<SessionId>,
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

impl ErrorClass {
    /// Stable snake-case name for logs, the attempts table and the UI.
    pub fn name(&self) -> &'static str {
        match self {
            Self::TransientRateLimit { .. } => "transient_rate_limit",
            Self::QuotaExhausted { .. } => "quota_exhausted",
            Self::Overloaded => "overloaded",
            Self::ContextOverflow { .. } => "context_overflow",
            Self::Auth => "auth",
            Self::ContentPolicy => "content_policy",
            Self::BadRequest { .. } => "bad_request",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
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
        let id = TerminalId::new();
        let json = serde_json::to_string(&AppEvent::TerminalExited {
            terminal_id: id,
            exit_code: Some(0),
        })
        .unwrap();
        assert_eq!(
            json,
            format!(r#"{{"type":"terminal_exited","terminal_id":"{id}","exit_code":0}}"#)
        );
    }
}
