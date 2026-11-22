//! Provider adapters. Each turns the canonical transcript into its wire format, streams the
//! response back as provider-neutral [`StreamEvent`]s, and classifies errors into
//! [`ErrorClass`]. Adapters never retry or switch: that is the router's job (`BUILD_PROMPT` §3.3).

mod anthropic;
mod classify;
mod ollama;
mod openai_chat;
mod openai_responses;
mod sse;

use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use termflow_types::{ErrorClass, ModelInfo, Part, StreamEvent, Turn, WireFormat};
use tokio_util::sync::CancellationToken;

pub use classify::{classify_http, retry_after_secs};

/// One concrete place to send a request.
#[derive(Clone)]
pub struct Target {
    /// Provider id (`anthropic`, `openai`, `openrouter`, `ollama`, `custom`).
    pub provider: String,
    pub wire: WireFormat,
    pub base_url: String,
    pub model: String,
    pub credential_id: Option<String>,
    pub api_key: Option<String>,
    /// No bytes for this long mid-stream counts as a stall (`Overloaded`).
    pub stall_timeout: Duration,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("provider", &self.provider)
            .field("wire", &self.wire)
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("credential_id", &self.credential_id)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}

/// A tool offered to the model (M2b fills these).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Everything rendering needs besides the transcript.
#[derive(Debug, Clone)]
pub struct RenderContext {
    pub model: String,
    pub system: String,
    pub max_tokens: u32,
    /// Ask for reasoning (adaptive thinking / reasoning summaries / `think`).
    pub reasoning: bool,
    /// Family tag stored on reasoning this target returns, and matched when re-sending it.
    pub family: String,
    /// Account the reasoning is bound to (we use the credential id).
    pub credential_scope: String,
    pub tools: Vec<ToolSpec>,
}

impl RenderContext {
    /// Native reasoning is re-sent only to the same family and credential scope (§7.1).
    pub fn may_resend(&self, p: &Part) -> bool {
        matches!(p, Part::Reasoning { family, credential_scope, .. }
            if *family == self.family && *credential_scope == self.credential_scope)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub class: ErrorClass,
    pub status: Option<u16>,
    /// Human-readable, from the provider's error body (never contains our API key).
    pub message: String,
}

impl ProviderError {
    pub fn new(class: ErrorClass, status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            class,
            status,
            message: message.into(),
        }
    }
}

pub type EventStream = BoxStream<'static, Result<StreamEvent, ProviderError>>;

#[async_trait::async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn wire(&self) -> WireFormat;
    /// Request body for `transcript` (committed turns only).
    fn render(&self, transcript: &[Turn], ctx: &RenderContext) -> Result<serde_json::Value, ProviderError>;
    /// Send `body` and stream events. HTTP errors come back as `Err` before any event.
    async fn stream(
        &self,
        target: &Target,
        body: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError>;
    async fn list_models(&self, target: &Target) -> Result<Vec<ModelInfo>, ProviderError>;
}

/// The adapter for a wire format, sharing one HTTP client.
pub fn adapter_for(wire: WireFormat, http: &reqwest::Client) -> Arc<dyn ProviderAdapter> {
    match wire {
        WireFormat::AnthropicMessages => Arc::new(anthropic::Anthropic::new(http.clone())),
        WireFormat::OpenaiResponses => Arc::new(openai_responses::OpenAiResponses::new(http.clone())),
        WireFormat::OpenaiChat => Arc::new(openai_chat::OpenAiChat::new(http.clone())),
        WireFormat::Ollama => Arc::new(ollama::Ollama::new(http.clone())),
    }
}

/// HTTP client with sane timeouts for streaming LLM calls.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("TermFlow/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

fn text_of(content: &[termflow_types::ToolResultContent]) -> String {
    content
        .iter()
        .map(|c| match c {
            termflow_types::ToolResultContent::Text { text } => text.as_str(),
        })
        .collect()
}

/// Map a `reqwest` transport error (connect, TLS, timeout, reset) to `Overloaded` (§7.4).
fn transport(e: &reqwest::Error) -> ProviderError {
    ProviderError::new(ErrorClass::Overloaded, None, format!("connection error: {e}"))
}

#[cfg(test)]
mod tests;
