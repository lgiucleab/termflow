//! The agent (`BUILD_PROMPT` §6.1): model → tool calls → permissions → execute → tool results →
//! repeat, until a final answer, Esc, or the step limit. Every model call goes through the
//! router; every tool call goes through `termflow-perms` first.

mod prompt;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use termflow_perms::{Decision, Mode, Rule, SessionGrants};
use termflow_providers::ToolSpec;
use termflow_router::{RouteError, RouteRequest, Router};
use termflow_store::Store;
use termflow_tools::{BlockLookup, Todo, Tool, ToolCtx, ToolOutput};
use termflow_types::{
    AgentSession, AppEvent, CallId, ErrorClass, Part, Role, SessionId, StreamEvent, ToolResultContent, Turn, TurnStatus,
};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

pub use prompt::INIT as INIT_PROMPT;

/// Output cap per request (per-model limits arrive with the M3 catalog).
const MAX_TOKENS: u32 = 16_000;
/// Model round-trips per user message (§6.1).
const MAX_STEPS: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("no such session")]
    NoSession,
    #[error("pick a model for this session first")]
    NoModel,
    #[error("the agent is already working in this session")]
    Busy,
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
    #[error(transparent)]
    Route(#[from] RouteError),
}

/// The user's answer on an approval card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Approval {
    AllowOnce,
    AllowSession,
    Deny { feedback: Option<String> },
}

/// Settings the core passes in.
#[derive(Clone, Default)]
pub struct ChatOptions {
    /// Permission rules in order (defaults first, then the user's config).
    pub rules: Vec<Rule>,
    /// Global instructions folder (`~/.config/termflow`).
    pub global_dir: Option<PathBuf>,
    pub blocks: Option<BlockLookup>,
    /// Shell for the bash tool (None = platform default).
    pub shell: Option<String>,
}

struct Shared {
    store: Store,
    router: Arc<Router>,
    bus: broadcast::Sender<AppEvent>,
    opts: ChatOptions,
    tools: Vec<Arc<dyn Tool>>,
    running: Mutex<HashMap<SessionId, CancellationToken>>,
    approvals: Mutex<HashMap<CallId, oneshot::Sender<Approval>>>,
    grants: Mutex<HashMap<SessionId, SessionGrants>>,
    todos: Mutex<HashMap<SessionId, Arc<Mutex<Vec<Todo>>>>>,
}

#[derive(Clone)]
pub struct Chat(Arc<Shared>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A poisoned lock only means a panic elsewhere; the data is still usable.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn mode_of(s: &AgentSession) -> Mode {
    if s.mode == "plan" { Mode::Plan } else { Mode::Build }
}

impl Chat {
    pub fn new(store: Store, router: Arc<Router>, bus: broadcast::Sender<AppEvent>, opts: ChatOptions) -> Self {
        Self(Arc::new(Shared {
            store,
            router,
            bus,
            opts,
            tools: termflow_tools::builtin(),
            running: Mutex::default(),
            approvals: Mutex::default(),
            grants: Mutex::default(),
            todos: Mutex::default(),
        }))
    }

    pub fn router(&self) -> &Router {
        &self.0.router
    }

    fn store(&self) -> &Store {
        &self.0.store
    }

    fn emit(&self, e: AppEvent) {
        let _ = self.0.bus.send(e); // no subscribers is fine
    }

    // ---- sessions ----

    pub async fn create_session(
        &self,
        workspace_id: Option<String>,
        credential_id: Option<String>,
        model: Option<String>,
    ) -> Result<AgentSession, AgentError> {
        let id = SessionId::new();
        self.store()
            .write(move |c| {
                termflow_store::agent::create_session(
                    c,
                    id,
                    workspace_id.as_deref(),
                    credential_id.as_deref(),
                    model.as_deref(),
                )
            })
            .await?;
        self.session(id)
    }

    pub fn sessions(&self, workspace_id: Option<&str>) -> Result<Vec<AgentSession>, AgentError> {
        Ok(termflow_store::agent::list_sessions(
            &self.store().reader()?,
            workspace_id,
        )?)
    }

    pub fn session(&self, id: SessionId) -> Result<AgentSession, AgentError> {
        termflow_store::agent::get_session(&self.store().reader()?, id)?.ok_or(AgentError::NoSession)
    }

    pub async fn set_model(
        &self,
        id: SessionId,
        credential_id: String,
        model: String,
        reasoning: bool,
    ) -> Result<(), AgentError> {
        self.store()
            .write(move |c| termflow_store::agent::set_session_model(c, id, &credential_id, &model, reasoning))
            .await?;
        Ok(())
    }

    pub async fn set_mode(&self, id: SessionId, plan: bool) -> Result<(), AgentError> {
        let mode = if plan { "plan" } else { "build" };
        self.store()
            .write(move |c| termflow_store::agent::set_session_mode(c, id, mode))
            .await?;
        Ok(())
    }

    pub async fn rename(&self, id: SessionId, title: String) -> Result<(), AgentError> {
        self.store()
            .write(move |c| termflow_store::agent::set_session_title(c, id, &title))
            .await?;
        Ok(())
    }

    pub async fn delete(&self, id: SessionId) -> Result<(), AgentError> {
        self.cancel(id);
        self.store()
            .write(move |c| termflow_store::agent::delete_session(c, id))
            .await?;
        Ok(())
    }

    pub fn transcript(&self, id: SessionId) -> Result<Vec<Turn>, AgentError> {
        Ok(termflow_store::agent::committed_turns(&self.store().reader()?, id)?)
    }

    pub fn usage(&self, id: SessionId) -> Result<(i64, i64, i64, i64, i64, i64), AgentError> {
        Ok(termflow_store::agent::session_usage(&self.store().reader()?, id)?)
    }

    pub fn is_running(&self, id: SessionId) -> bool {
        lock(&self.0.running).contains_key(&id)
    }

    /// Stop the agent in `id`: the stream, any running tool, and pending approvals.
    pub fn cancel(&self, id: SessionId) {
        if let Some(t) = lock(&self.0.running).get(&id) {
            t.cancel();
        }
    }

    /// Answer an approval card. Returns false if nothing was waiting for `call`.
    pub fn approve(&self, call: CallId, decision: Approval) -> bool {
        lock(&self.0.approvals)
            .remove(&call)
            .is_some_and(|tx| tx.send(decision).is_ok())
    }

    fn root(&self, s: &AgentSession) -> Option<PathBuf> {
        let wid = s.workspace_id.as_deref()?;
        let w = termflow_store::agent::get_workspace(&self.store().reader().ok()?, wid).ok()??;
        PathBuf::from(w.path).canonicalize().ok()
    }

    /// Commit the user's message (expanding `@file` mentions), then run the agent in the background.
    pub async fn send(&self, id: SessionId, text: String) -> Result<(), AgentError> {
        let session = self.session(id)?;
        let (Some(credential_id), Some(model)) = (session.credential_id.clone(), session.model.clone()) else {
            return Err(AgentError::NoModel);
        };
        let cancel = CancellationToken::new();
        {
            let mut running = lock(&self.0.running);
            if running.contains_key(&id) {
                return Err(AgentError::Busy);
            }
            running.insert(id, cancel.clone());
        }
        let root = self.root(&session);
        let expanded = root
            .as_deref()
            .map_or_else(|| text.clone(), |r| termflow_context::expand_mentions(r, &text));
        let first = session.title.is_none();
        let res = async {
            // A tool call left without a result (app closed mid-run) gets one before anything else.
            self.repair(id).await?;
            let user = Turn::new(
                id,
                Role::User,
                vec![Part::Text { text: expanded }],
                TurnStatus::Committed,
            );
            let u = user.clone();
            self.store()
                .write(move |c| {
                    termflow_store::agent::append_turn(c, &u)?;
                    if first {
                        let title: String = text.lines().next().unwrap_or_default().chars().take(60).collect();
                        termflow_store::agent::set_session_title(c, id, &title)?;
                    }
                    Ok(())
                })
                .await?;
            Ok::<_, AgentError>(user)
        }
        .await;
        let user = match res {
            Ok(u) => u,
            Err(e) => {
                lock(&self.0.running).remove(&id);
                return Err(e);
            }
        };
        self.emit(AppEvent::AgentTurnCommitted { turn: user });
        let me = self.clone();
        tokio::spawn(async move {
            me.run(session, credential_id, model, root, cancel).await;
            lock(&me.0.running).remove(&id);
            lock(&me.0.grants).entry(id).or_default(); // keep session grants for the next message
            me.emit(AppEvent::AgentIdle { session_id: id });
        });
        Ok(())
    }

    /// Give every tool call without a result an error result, so the transcript stays valid.
    /// Never re-runs anything (§7.7.3).
    async fn repair(&self, id: SessionId) -> Result<(), AgentError> {
        let turns = self.transcript(id)?;
        let answered: HashSet<CallId> = turns
            .iter()
            .flat_map(|t| &t.parts)
            .filter_map(|p| {
                if let Part::ToolResult { call_id, .. } = p {
                    Some(*call_id)
                } else {
                    None
                }
            })
            .collect();
        let missing: Vec<Part> = turns
            .iter()
            .flat_map(|t| &t.parts)
            .filter_map(|p| match p {
                Part::ToolCall { call_id, .. } if !answered.contains(call_id) => Some(Part::ToolResult {
                    call_id: *call_id,
                    content: vec![ToolResultContent::Text {
                        text: "Interrupted: the app stopped before this tool finished. It was not re-run.".into(),
                    }],
                    is_error: true,
                }),
                _ => None,
            })
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let t = Turn::new(id, Role::Tool, missing, TurnStatus::Committed);
        let t2 = t.clone();
        self.store()
            .write(move |c| termflow_store::agent::append_turn(c, &t2))
            .await?;
        self.emit(AppEvent::AgentTurnCommitted { turn: t });
        Ok(())
    }

    fn tool_specs(&self) -> Vec<ToolSpec> {
        self.0
            .tools
            .iter()
            .map(|t| ToolSpec {
                name: t.name().into(),
                description: t.description().into(),
                parameters: t.parameters(),
            })
            .collect()
    }

    async fn run(
        &self,
        session: AgentSession,
        credential_id: String,
        model: String,
        root: Option<PathBuf>,
        cancel: CancellationToken,
    ) {
        let id = session.id;
        let mode = mode_of(&session);
        let system = match &root {
            Some(r) => {
                let shell = self.0.opts.shell.clone().unwrap_or_else(|| default_shell().into());
                let env = termflow_context::environment(r, &shell).await;
                let files = termflow_context::discover(r, self.0.opts.global_dir.as_deref());
                prompt::assemble(mode, Some(&env), &files)
            }
            None => prompt::assemble(mode, None, &[]),
        };
        let tools = if root.is_some() { self.tool_specs() } else { vec![] };
        for step in 0..MAX_STEPS {
            let Some(turn) = self
                .model_turn(
                    id,
                    &credential_id,
                    &model,
                    session.reasoning,
                    &system,
                    tools.clone(),
                    &cancel,
                )
                .await
            else {
                return; // error, cancel, or no output: already reported
            };
            let calls: Vec<(CallId, String, serde_json::Value)> = turn
                .parts
                .iter()
                .filter_map(|p| {
                    if let Part::ToolCall { call_id, name, args } = p {
                        Some((*call_id, name.clone(), args.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            if calls.is_empty() || cancel.is_cancelled() {
                return;
            }
            let Some(root) = &root else { return };
            let results = self.execute(id, mode, root, &calls, &cancel).await;
            let t = Turn::new(id, Role::Tool, results, TurnStatus::Committed);
            let t2 = t.clone();
            if let Err(e) = self
                .store()
                .write(move |c| termflow_store::agent::append_turn(c, &t2))
                .await
            {
                return self.fail(id, None, &ErrorClass::Unknown, &e.to_string());
            }
            self.emit(AppEvent::AgentTurnCommitted { turn: t });
            if cancel.is_cancelled() {
                return;
            }
            if step + 1 == MAX_STEPS {
                self.fail(
                    id,
                    None,
                    &ErrorClass::Unknown,
                    &format!("stopped after {MAX_STEPS} steps; send a message to continue"),
                );
            }
        }
    }

    /// One model round-trip. Returns the committed assistant turn, or None if it failed.
    // One linear streaming flow (send → accumulate → commit/discard); splitting it would scatter it.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn model_turn(
        &self,
        id: SessionId,
        credential_id: &str,
        model: &str,
        reasoning: bool,
        system: &str,
        tools: Vec<ToolSpec>,
        cancel: &CancellationToken,
    ) -> Option<Turn> {
        let transcript = match self.transcript(id) {
            Ok(t) => t,
            Err(e) => {
                self.fail(id, None, &ErrorClass::Unknown, &e.to_string());
                return None;
            }
        };
        let mut turn = Turn::new(id, Role::Assistant, vec![], TurnStatus::Partial);
        let t = turn.clone();
        if let Err(e) = self
            .store()
            .write(move |c| termflow_store::agent::append_turn(c, &t))
            .await
        {
            self.fail(id, None, &ErrorClass::Unknown, &e.to_string());
            return None;
        }
        let req = RouteRequest {
            session: id,
            turn: turn.id,
            credential_id: credential_id.into(),
            model: model.into(),
            transcript,
            system: system.into(),
            max_tokens: MAX_TOKENS,
            reasoning,
            tools,
        };
        let mut route = match self.0.router.stream(req, cancel.clone()).await {
            Ok(r) => r,
            Err(e) => {
                self.discard(&turn, e.class(), &e.to_string()).await;
                return None;
            }
        };
        self.emit(AppEvent::AgentTurnStarted {
            session_id: id,
            turn_id: turn.id,
            model: route.served.model.clone(),
        });
        let mut b = PartsBuilder {
            family: route.family.clone(),
            scope: route.credential_scope.clone(),
            model: model.into(),
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
                        self.emit(AppEvent::AgentDelta {
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
        let natives = std::mem::take(&mut b.natives);
        turn.parts = b.finish();
        turn.served_by = Some(route.served);
        match outcome {
            Ok(()) => {
                let calls: Vec<(CallId, String, String, serde_json::Value)> = turn
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { call_id, name, args } => Some((
                            *call_id,
                            natives.get(call_id).cloned().unwrap_or_default(),
                            name.clone(),
                            args.clone(),
                        )),
                        _ => None,
                    })
                    .collect();
                self.commit(turn, calls, credential_id).await
            }
            // Esc: keep what was written as plain text (no half reasoning/tool calls) (§6.1).
            Err((ErrorClass::Cancelled, _)) => {
                turn.parts.retain(|p| matches!(p, Part::Text { .. }));
                if turn.parts.is_empty() {
                    self.discard(&turn, ErrorClass::Cancelled, "cancelled").await;
                } else {
                    turn.parts.push(Part::Text {
                        text: "\n\n[cancelled]".into(),
                    });
                    self.commit(turn, vec![], credential_id).await;
                }
                None
            }
            Err((class, msg)) => {
                self.discard(&turn, class, &msg).await;
                None
            }
        }
    }

    async fn commit(
        &self,
        mut turn: Turn,
        calls: Vec<(CallId, String, String, serde_json::Value)>,
        issuer: &str,
    ) -> Option<Turn> {
        turn.status = TurnStatus::Committed;
        let (t, issuer) = (turn.clone(), issuer.to_owned());
        let res = self
            .store()
            .write(move |c| {
                termflow_store::agent::commit_turn(c, &t)?;
                if !calls.is_empty() {
                    termflow_store::agent::insert_tool_calls(c, t.session_id, t.id, &issuer, &calls)?;
                }
                Ok(())
            })
            .await;
        match res {
            Ok(()) => {
                self.emit(AppEvent::AgentTurnCommitted { turn: turn.clone() });
                Some(turn)
            }
            Err(e) => {
                self.fail(turn.session_id, Some(turn.id), &ErrorClass::Unknown, &e.to_string());
                None
            }
        }
    }

    /// Run a batch of tool calls: read-only ones concurrently, mutating ones one at a time in
    /// order (§6.1). Results come back in the calls' order.
    async fn execute(
        &self,
        id: SessionId,
        mode: Mode,
        root: &std::path::Path,
        calls: &[(CallId, String, serde_json::Value)],
        cancel: &CancellationToken,
    ) -> Vec<Part> {
        let todos = lock(&self.0.todos).entry(id).or_default().clone();
        let ctx = ToolCtx {
            root: root.to_path_buf(),
            cancel: cancel.child_token(),
            todos,
            blocks: self.0.opts.blocks.clone(),
            shell: self.0.opts.shell.clone(),
        };
        let mut results: Vec<Option<Part>> = vec![None; calls.len()];
        let mut batch: Vec<usize> = Vec::new();
        for (i, (_, name, args)) in calls.iter().enumerate() {
            let mutating = self.tool(name).is_none_or(|t| t.facts(args, &ctx).mutating);
            if mutating {
                self.run_batch(id, mode, &ctx, calls, &mut batch, &mut results).await;
                results[i] = Some(self.call(id, mode, &ctx, &calls[i]).await);
            } else {
                batch.push(i);
            }
        }
        self.run_batch(id, mode, &ctx, calls, &mut batch, &mut results).await;
        results.into_iter().flatten().collect()
    }

    async fn run_batch(
        &self,
        id: SessionId,
        mode: Mode,
        ctx: &ToolCtx,
        calls: &[(CallId, String, serde_json::Value)],
        batch: &mut Vec<usize>,
        results: &mut [Option<Part>],
    ) {
        let outs = join_all(batch.iter().map(|&i| self.call(id, mode, ctx, &calls[i]))).await;
        for (i, out) in batch.drain(..).zip(outs) {
            results[i] = Some(out);
        }
    }

    fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.0.tools.iter().find(|t| t.name() == name).cloned()
    }

    /// Permissions → (approval) → execute one call, exactly once.
    #[allow(clippy::too_many_lines)] // the call's whole lifecycle, read top to bottom
    async fn call(
        &self,
        id: SessionId,
        mode: Mode,
        ctx: &ToolCtx,
        (call_id, name, args): &(CallId, String, serde_json::Value),
    ) -> Part {
        let call_id = *call_id;
        let result = |text: String, is_error: bool| Part::ToolResult {
            call_id,
            content: vec![ToolResultContent::Text { text }],
            is_error,
        };
        // Never execute a call twice (e.g. after a model switch or a restart).
        let already = self
            .store()
            .reader()
            .ok()
            .and_then(|r| termflow_store::agent::exec_count(&r, call_id).ok())
            .unwrap_or(0);
        if already > 0 {
            return result(
                "This tool call was already executed earlier; it was not run again.".into(),
                true,
            );
        }
        let Some(tool) = self.tool(name) else {
            return result(format!("Unknown tool `{name}`."), true);
        };
        if ctx.cancel.is_cancelled() {
            return result("Cancelled by the user before running.".into(), true);
        }
        let facts = tool.facts(args, ctx);
        let decision = {
            let grants = lock(&self.0.grants).get(&id).cloned().unwrap_or_default();
            termflow_perms::evaluate(&self.0.opts.rules, &grants, mode, name, &facts, &ctx.root)
        };
        match decision {
            Decision::Deny(msg) => {
                let _ = self
                    .store()
                    .write(move |c| termflow_store::agent::finish_tool(c, call_id, "denied", ""))
                    .await;
                return result(msg, true);
            }
            Decision::Ask => {
                let preview = tool.preview(args, ctx).await;
                let (tx, rx) = oneshot::channel();
                lock(&self.0.approvals).insert(call_id, tx);
                self.emit(AppEvent::ToolApprovalNeeded {
                    session_id: id,
                    call_id,
                    tool: name.clone(),
                    summary: facts.summary.clone(),
                    preview,
                });
                let answer = tokio::select! {
                    a = rx => a.ok(),
                    () = ctx.cancel.cancelled() => None,
                };
                lock(&self.0.approvals).remove(&call_id);
                match answer {
                    Some(Approval::AllowOnce) => {}
                    Some(Approval::AllowSession) => lock(&self.0.grants).entry(id).or_default().grant(name, &facts),
                    Some(Approval::Deny { feedback }) => {
                        let _ = self
                            .store()
                            .write(move |c| termflow_store::agent::finish_tool(c, call_id, "denied", ""))
                            .await;
                        let fb = feedback
                            .filter(|f| !f.trim().is_empty())
                            .map_or(String::new(), |f| format!(" User's feedback: {f}"));
                        return result(format!("The user denied this tool call.{fb}"), true);
                    }
                    None => return result("Cancelled by the user before running.".into(), true),
                }
            }
            Decision::Allow => {}
        }
        if self
            .store()
            .write(move |c| termflow_store::agent::begin_tool_exec(c, call_id))
            .await
            .unwrap_or(1)
            > 1
        {
            return result(
                "This tool call was already executed earlier; it was not run again.".into(),
                true,
            );
        }
        self.emit(AppEvent::ToolRunning {
            session_id: id,
            call_id,
        });
        let out = match tokio::time::timeout(tool.timeout() + Duration::from_secs(5), tool.run(args.clone(), ctx)).await
        {
            Ok(o) => o,
            Err(_) => ToolOutput::err(format!("`{name}` timed out")),
        };
        let (status, full) = (if out.is_error { "error" } else { "done" }, out.full.clone());
        if let Err(e) = self
            .store()
            .write(move |c| termflow_store::agent::finish_tool(c, call_id, status, &full))
            .await
        {
            tracing::warn!(error = %e, "recording tool result failed");
        }
        result(out.model_text, out.is_error)
    }

    async fn discard(&self, turn: &Turn, class: ErrorClass, msg: &str) {
        let id = turn.id;
        if let Err(e) = self
            .store()
            .write(move |c| termflow_store::agent::set_turn_status(c, id, TurnStatus::Discarded))
            .await
        {
            tracing::warn!(error = %e, "discarding turn failed");
        }
        self.fail(turn.session_id, Some(turn.id), &class, msg);
    }

    fn fail(&self, session_id: SessionId, turn_id: Option<termflow_types::TurnId>, class: &ErrorClass, msg: &str) {
        tracing::info!(session = %session_id, class = class.name(), %msg, "agent turn failed");
        self.emit(AppEvent::AgentError {
            session_id,
            turn_id,
            class: class.name().into(),
            message: msg.into(),
        });
    }
}

fn default_shell() -> &'static str {
    if cfg!(windows) { "pwsh" } else { "/bin/bash" }
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
    /// our id → native id
    natives: HashMap<CallId, String>,
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
                    let args = if args.trim().is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::from_str(&args).unwrap_or(serde_json::Value::String(args))
                    };
                    self.natives.insert(call_id, native_id);
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
