//! Every model call goes through here (`BUILD_PROMPT` §3.2).
//!
//! M2: a pass-through to one target, with the attempt log. M3 adds health, budgets, fallback
//! chains, hysteresis and hand-off on top of this same entry point.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::stream::BoxStream;
use termflow_providers::{ProviderAdapter, ProviderError, RenderContext, Target, ToolSpec};
use termflow_secrets::Secrets;
use termflow_store::Store;
use termflow_store::agent::{AttemptEnd, AttemptStart};
use termflow_types::{
    AttemptId, Credential, ErrorClass, ModelInfo, ServedBy, SessionId, StreamEvent, Turn, TurnId, Usage, WireFormat,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const STALL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("no such credential: {0}")]
    NoCredential(String),
    #[error("the API key for this credential is missing from the keychain")]
    MissingSecret,
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
    #[error(transparent)]
    Secret(#[from] termflow_secrets::SecretError),
    #[error("{0}")]
    Provider(#[from] ProviderError),
}

impl RouteError {
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Provider(p) => p.class.clone(),
            Self::MissingSecret => ErrorClass::Auth,
            _ => ErrorClass::Unknown,
        }
    }
}

pub struct RouteRequest {
    pub session: SessionId,
    pub turn: TurnId,
    pub credential_id: String,
    pub model: String,
    pub transcript: Vec<Turn>,
    pub system: String,
    pub max_tokens: u32,
    pub reasoning: bool,
    pub tools: Vec<ToolSpec>,
}

/// A live response. `served` names who is answering; events end with `Done` or an error.
pub struct RouteStream {
    pub served: ServedBy,
    /// Family/scope to tag reasoning parts with.
    pub family: String,
    pub credential_scope: String,
    pub events: BoxStream<'static, Result<StreamEvent, ProviderError>>,
}

pub struct Router {
    store: Store,
    secrets: Secrets,
    adapters: HashMap<WireFormat, Arc<dyn ProviderAdapter>>,
}

impl Router {
    pub fn new(store: Store, secrets: Secrets) -> Self {
        let http = termflow_providers::http_client();
        let adapters = [
            WireFormat::AnthropicMessages,
            WireFormat::OpenaiResponses,
            WireFormat::OpenaiChat,
            WireFormat::Ollama,
        ]
        .into_iter()
        .map(|w| (w, termflow_providers::adapter_for(w, &http)))
        .collect();
        Self {
            store,
            secrets,
            adapters,
        }
    }

    fn credential(&self, id: &str) -> Result<Credential, RouteError> {
        termflow_store::agent::get_credential(&self.store.reader()?, id)?
            .ok_or_else(|| RouteError::NoCredential(id.into()))
    }

    fn target(&self, cred: &Credential, model: &str) -> Result<Target, RouteError> {
        let api_key = if cred.has_secret {
            Some(self.secrets.get(&cred.id)?.ok_or(RouteError::MissingSecret)?)
        } else {
            None
        };
        Ok(Target {
            provider: cred.kind.id().into(),
            wire: cred.wire,
            base_url: cred.base_url.clone(),
            model: model.into(),
            credential_id: Some(cred.id.clone()),
            api_key,
            stall_timeout: STALL_TIMEOUT,
        })
    }

    fn adapter(&self, w: WireFormat) -> Arc<dyn ProviderAdapter> {
        // Every WireFormat is registered in `new`.
        self.adapters
            .get(&w)
            .cloned()
            .unwrap_or_else(|| termflow_providers::adapter_for(w, &termflow_providers::http_client()))
    }

    pub async fn list_models(&self, credential_id: &str) -> Result<Vec<ModelInfo>, RouteError> {
        let cred = self.credential(credential_id)?;
        let target = self.target(&cred, "")?;
        Ok(self.adapter(cred.wire).list_models(&target).await?)
    }

    /// Send one request. Records an attempt row from start to finish, including failures.
    pub async fn stream(&self, req: RouteRequest, cancel: CancellationToken) -> Result<RouteStream, RouteError> {
        let cred = self.credential(&req.credential_id)?;
        let target = self.target(&cred, &req.model)?;
        let adapter = self.adapter(cred.wire);
        let family = cred.kind.id().to_owned();
        let ctx = RenderContext {
            model: req.model.clone(),
            system: req.system,
            max_tokens: req.max_tokens,
            reasoning: req.reasoning,
            family: family.clone(),
            credential_scope: cred.id.clone(),
            tools: req.tools,
        };
        let body = adapter.render(&req.transcript, &ctx)?;

        let attempt = AttemptId::new();
        let (provider, model, cred_id) = (target.provider.clone(), req.model.clone(), cred.id.clone());
        {
            let (p, m, c) = (provider.clone(), model.clone(), cred_id.clone());
            let (session, turn) = (req.session, req.turn);
            self.store
                .write(move |conn| {
                    termflow_store::agent::start_attempt(
                        conn,
                        &AttemptStart {
                            id: attempt,
                            session,
                            turn,
                            provider: &p,
                            model: &m,
                            credential_id: Some(&c),
                            retry_no: 0,
                        },
                    )
                })
                .await?;
        }
        let started = Instant::now();
        let served = ServedBy {
            provider,
            model,
            credential_id: Some(cred_id.clone()),
            attempt_id: attempt,
        };

        let inner = match adapter.stream(&target, body, cancel).await {
            Ok(s) => s,
            Err(e) => {
                finish_attempt(
                    &self.store,
                    attempt,
                    e.status,
                    Some(e.class.clone()),
                    None,
                    Usage::default(),
                    None,
                )
                .await;
                return Err(e.into());
            }
        };
        let (tx, rx) = mpsc::channel(256);
        let store = self.store.clone();
        tokio::spawn(async move {
            let mut inner = inner;
            let mut usage = Usage::default();
            let mut ttft: Option<u64> = None;
            let mut served_model: Option<String> = None;
            let mut error: Option<ErrorClass> = None;
            while let Some(ev) = inner.next().await {
                match &ev {
                    Ok(
                        StreamEvent::TextDelta(_) | StreamEvent::ReasoningDelta(_) | StreamEvent::ToolCallStart { .. },
                    ) => {
                        ttft.get_or_insert_with(|| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
                    }
                    Ok(StreamEvent::Usage(u)) => usage.merge(*u),
                    Ok(StreamEvent::ServedModel(m)) => served_model = Some(m.clone()),
                    Err(e) => error = Some(e.class.clone()),
                    _ => {}
                }
                if tx.send(ev).await.is_err() {
                    error.get_or_insert(ErrorClass::Cancelled);
                    break;
                }
            }
            finish_attempt(&store, attempt, Some(200), error, ttft, usage, served_model).await;
        });
        Ok(RouteStream {
            served,
            family,
            credential_scope: cred_id,
            events: tokio_stream::wrappers::ReceiverStream::new(rx).boxed(),
        })
    }
}

/// Close an attempt row (success or failure). Logging only: never fails the request.
async fn finish_attempt(
    store: &Store,
    id: AttemptId,
    status: Option<u16>,
    error: Option<ErrorClass>,
    ttft: Option<u64>,
    usage: Usage,
    served_model: Option<String>,
) {
    let res = store
        .write(move |c| {
            termflow_store::agent::finish_attempt(
                c,
                id,
                &AttemptEnd {
                    http_status: status,
                    error: error.as_ref(),
                    ttft_ms: ttft,
                    usage,
                    served_model: served_model.as_deref(),
                },
            )
        })
        .await;
    if let Err(e) = res {
        tracing::warn!(error = %e, "recording attempt failed");
    }
}

#[cfg(test)]
mod tests;
