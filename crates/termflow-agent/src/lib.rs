//! Agent sessions. M2a: a streaming chat over the canonical transcript, one model per session,
//! every call through the router. M2b adds tools, permissions and the agent loop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use termflow_router::{RouteError, RouteRequest, Router};
use termflow_store::Store;
use termflow_types::{
    AgentSession, AppEvent, CallId, ErrorClass, Part, Role, SessionId, StreamEvent, Turn, TurnStatus,
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

pub const BASE_PROMPT: &str = include_str!("../prompts/system/base.md");
/// Output cap per request. Safe for every current provider we target; per-model limits arrive
/// with the M3 catalog.
const MAX_TOKENS: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("no such session")]
    NoSession,
    #[error("pick a model for this session first")]
    NoModel,
    #[error("a reply is already streaming in this session")]
    Busy,
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
    #[error(transparent)]
    Route(#[from] RouteError),
}

#[derive(Clone)]
pub struct Chat {
    store: Store,
    router: Arc<Router>,
    bus: broadcast::Sender<AppEvent>,
    running: Arc<Mutex<HashMap<SessionId, CancellationToken>>>,
}

impl Chat {
    pub fn new(store: Store, router: Arc<Router>, bus: broadcast::Sender<AppEvent>) -> Self {
        Self {
            store,
            router,
            bus,
            running: Arc::default(),
        }
    }

    pub fn router(&self) -> &Router {
        &self.router
    }

    pub async fn create_session(
        &self,
        credential_id: Option<String>,
        model: Option<String>,
    ) -> Result<AgentSession, AgentError> {
        let id = SessionId::new();
        self.store
            .write(move |c| termflow_store::agent::create_session(c, id, credential_id.as_deref(), model.as_deref()))
            .await?;
        termflow_store::agent::get_session(&self.store.reader()?, id)?.ok_or(AgentError::NoSession)
    }

    pub fn sessions(&self) -> Result<Vec<AgentSession>, AgentError> {
        Ok(termflow_store::agent::list_sessions(&self.store.reader()?)?)
    }

    pub fn session(&self, id: SessionId) -> Result<AgentSession, AgentError> {
        termflow_store::agent::get_session(&self.store.reader()?, id)?.ok_or(AgentError::NoSession)
    }

    pub async fn set_model(
        &self,
        id: SessionId,
        credential_id: String,
        model: String,
        reasoning: bool,
    ) -> Result<(), AgentError> {
        self.store
            .write(move |c| termflow_store::agent::set_session_model(c, id, &credential_id, &model, reasoning))
            .await?;
        Ok(())
    }

    pub fn transcript(&self, id: SessionId) -> Result<Vec<Turn>, AgentError> {
        Ok(termflow_store::agent::committed_turns(&self.store.reader()?, id)?)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, CancellationToken>> {
        self.running.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Stop the reply streaming in `id`, if any. Partial text is kept (§6.1 cancellation).
    pub fn cancel(&self, id: SessionId) {
        if let Some(t) = self.lock().get(&id) {
            t.cancel();
        }
    }

    /// Commit the user's message, then stream the reply in the background.
    pub async fn send(&self, id: SessionId, text: String) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let (Some(credential_id), Some(model)) = (session.credential_id, session.model) else {
            return Err(AgentError::NoModel);
        };
        let cancel = CancellationToken::new();
        {
            let mut running = self.lock();
            if running.contains_key(&id) {
                return Err(AgentError::Busy);
            }
            running.insert(id, cancel.clone());
        }
        let user = Turn::new(
            id,
            Role::User,
            vec![Part::Text { text: text.clone() }],
            TurnStatus::Committed,
        );
        let u = user.clone();
        let first = session.title.is_none();
        let res = self
            .store
            .write(move |c| {
                termflow_store::agent::append_turn(c, &u)?;
                if first {
                    let title: String = text.lines().next().unwrap_or_default().chars().take(60).collect();
                    termflow_store::agent::set_session_title(c, id, &title)?;
                }
                Ok(())
            })
            .await;
        if let Err(e) = res {
            self.lock().remove(&id);
            return Err(e.into());
        }
        let _ = self.bus.send(AppEvent::AgentTurnCommitted { turn: user });

        let me = self.clone();
        tokio::spawn(async move {
            me.reply(id, credential_id, model, session.reasoning, cancel).await;
            me.lock().remove(&id);
        });
        Ok(())
    }

    async fn reply(
        &self,
        id: SessionId,
        credential_id: String,
        model: String,
        reasoning: bool,
        cancel: CancellationToken,
    ) {
        let transcript = match self.transcript(id) {
            Ok(t) => t,
            Err(e) => return self.fail(id, None, &ErrorClass::Unknown, &e.to_string()),
        };
        let mut turn = Turn::new(id, Role::Assistant, vec![], TurnStatus::Partial);
        let t = turn.clone();
        if let Err(e) = self
            .store
            .write(move |c| termflow_store::agent::append_turn(c, &t))
            .await
        {
            return self.fail(id, None, &ErrorClass::Unknown, &e.to_string());
        }
        let req = RouteRequest {
            session: id,
            turn: turn.id,
            credential_id,
            model: model.clone(),
            transcript,
            system: BASE_PROMPT.trim().to_owned(),
            max_tokens: MAX_TOKENS,
            reasoning,
            tools: vec![],
        };
        let mut route = match self.router.stream(req, cancel.clone()).await {
            Ok(r) => r,
            Err(e) => return self.discard(&turn, e.class(), &e.to_string()).await,
        };
        let _ = self.bus.send(AppEvent::AgentTurnStarted {
            session_id: id,
            turn_id: turn.id,
            model: route.served.model.clone(),
        });

        let mut b = PartsBuilder {
            family: route.family.clone(),
            scope: route.credential_scope.clone(),
            model,
            ..Default::default()
        };
        let mut outcome: Result<(), (ErrorClass, String)> = Err((ErrorClass::Unknown, "stream ended early".into()));
        while let Some(ev) = route.events.next().await {
            match ev {
                Ok(StreamEvent::Done { .. }) => {
                    outcome = Ok(());
                    break;
                }
                Ok(StreamEvent::ServedModel(m)) => route.served.model = m,
                Ok(ev) => {
                    if let Some((text, reasoning)) = b.push(ev) {
                        let _ = self.bus.send(AppEvent::AgentDelta {
                            session_id: id,
                            turn_id: turn.id,
                            text,
                            reasoning,
                        });
                    }
                }
                Err(e) => {
                    outcome = Err((e.class, e.message));
                    break;
                }
            }
        }
        turn.parts = b.finish();
        turn.served_by = Some(route.served);
        match outcome {
            Ok(()) => self.commit(turn).await,
            // Esc: keep what was written as plain text (no half reasoning/tool calls).
            Err((ErrorClass::Cancelled, _)) => {
                turn.parts.retain(|p| matches!(p, Part::Text { .. }));
                if turn.parts.is_empty() {
                    self.discard(&turn, ErrorClass::Cancelled, "cancelled").await;
                } else {
                    turn.parts.push(Part::Text {
                        text: "\n\n[cancelled]".into(),
                    });
                    self.commit(turn).await;
                }
            }
            Err((class, msg)) => self.discard(&turn, class, &msg).await,
        }
    }

    async fn commit(&self, mut turn: Turn) {
        turn.status = TurnStatus::Committed;
        let t = turn.clone();
        match self
            .store
            .write(move |c| termflow_store::agent::commit_turn(c, &t))
            .await
        {
            Ok(()) => {
                let _ = self.bus.send(AppEvent::AgentTurnCommitted { turn });
            }
            Err(e) => self.fail(turn.session_id, Some(turn.id), &ErrorClass::Unknown, &e.to_string()),
        }
    }

    /// Mid-stream failure: the partial turn is kept in the DB as `discarded`, never resent (§7.7).
    async fn discard(&self, turn: &Turn, class: ErrorClass, msg: &str) {
        let id = turn.id;
        if let Err(e) = self
            .store
            .write(move |c| termflow_store::agent::set_turn_status(c, id, TurnStatus::Discarded))
            .await
        {
            tracing::warn!(error = %e, "discarding turn failed");
        }
        self.fail(turn.session_id, Some(turn.id), &class, msg);
    }

    fn fail(&self, session_id: SessionId, turn_id: Option<termflow_types::TurnId>, class: &ErrorClass, msg: &str) {
        tracing::info!(session = %session_id, class = class.name(), %msg, "agent turn failed");
        let _ = self.bus.send(AppEvent::AgentError {
            session_id,
            turn_id,
            class: class.name().into(),
            message: msg.into(),
        });
    }
}

/// Assembles stream events into transcript parts, in the order they arrived.
#[derive(Default)]
struct PartsBuilder {
    family: String,
    scope: String,
    model: String,
    parts: Vec<Part>,
    /// native id → (our id, name, args so far)
    calls: HashMap<String, (CallId, String, String)>,
}

impl PartsBuilder {
    /// Returns display text for the UI: `(text, is_reasoning)`.
    fn push(&mut self, ev: StreamEvent) -> Option<(String, bool)> {
        match ev {
            StreamEvent::TextDelta(t) => {
                match self.parts.last_mut() {
                    Some(Part::Text { text }) => text.push_str(&t),
                    _ => self.parts.push(Part::Text { text: t.clone() }),
                }
                Some((t, false))
            }
            StreamEvent::ReasoningDelta(t) => Some((t, true)),
            StreamEvent::ReasoningDone { native, summary_text } => {
                self.parts.push(Part::Reasoning {
                    family: self.family.clone(),
                    model: self.model.clone(),
                    credential_scope: self.scope.clone(),
                    native,
                    summary_text,
                });
                None
            }
            StreamEvent::ToolCallStart { native_id, name } => {
                self.calls.insert(native_id, (CallId::new(), name, String::new()));
                None
            }
            StreamEvent::ToolCallArgsDelta {
                native_id,
                json_fragment,
            } => {
                if let Some(c) = self.calls.get_mut(&native_id) {
                    c.2.push_str(&json_fragment);
                }
                None
            }
            StreamEvent::ToolCallEnd { native_id } => {
                if let Some((call_id, name, args)) = self.calls.remove(&native_id) {
                    let args = serde_json::from_str(&args).unwrap_or(serde_json::Value::String(args));
                    self.parts.push(Part::ToolCall { call_id, name, args });
                }
                None
            }
            StreamEvent::Usage(_)
            | StreamEvent::RateLimit(_)
            | StreamEvent::ServedModel(_)
            | StreamEvent::Done { .. } => None,
        }
    }

    fn finish(self) -> Vec<Part> {
        self.parts
    }
}

#[cfg(test)]
mod tests;
