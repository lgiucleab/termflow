//! Every model call goes through here (`BUILD_PROMPT` §3.2, §7).
//!
//! A request names a pinned model or a fallback profile. The router orders the candidates
//! (capabilities, health, budget, context fit, rate-limit headers, hysteresis), retries or
//! moves on per the §7.6 action table, and only hands the agent a stream once real content
//! has arrived. A failure after that is a mid-stream hand-off: the agent discards the partial
//! turn and asks again with that target excluded.

pub mod clock;
pub mod estimate;
pub mod health;
pub mod policy;
pub mod select;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::stream::BoxStream;
use jiff::Timestamp;
use termflow_catalog::Catalog;
use termflow_config::{Profile, SwitchBack};
use termflow_providers::{EventStream, ProviderAdapter, ProviderError, RenderContext, Target, ToolSpec};
use termflow_secrets::Secrets;
use termflow_store::Store;
use termflow_store::agent::{AttemptEnd, AttemptStart};
use termflow_types::{
    AppEvent, AttemptId, CandidateState, Credential, ErrorClass, ModelInfo, RateLimitSnapshot, Role, ServedBy,
    SessionId, StreamEvent, Turn, TurnId, Usage, WireFormat,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::clock::{Clock, SystemClock, until};
use crate::health::{CredentialBlock, HealthRegistry, HealthState, TargetKey};
use crate::policy::{Action, Cool, RetryPolicy, decide};
use crate::select::Candidate;

const STALL_TIMEOUT: Duration = Duration::from_secs(120);
const RATE_LIMIT_TTL: Duration = Duration::from_secs(60);
/// How many times an all-down session waits for a reset and tries again by itself.
const MAX_PAUSES: u32 = 3;
/// Recent turns kept intact when trimming tool output to fit a smaller context.
const KEEP_LAST_TURNS: usize = 6;
const HANDOFF_NOTE: &str = "\n\nNote: you are continuing a session that another model started. \
The conversation above is real: tool calls already ran and their results are final, so don't \
run them again unless the user asks. Continue from the last message.";

#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("no such credential: {0}")]
    NoCredential(String),
    #[error("no such profile: {0}")]
    NoProfile(String),
    #[error("the API key for this credential is missing from the keychain")]
    MissingSecret,
    #[error("all models in this profile are unavailable")]
    AllUnavailable {
        candidates: Vec<CandidateState>,
        resume_at: Option<Timestamp>,
    },
    #[error("{scope} budget reached: ${spent:.2} of ${limit:.2}")]
    BudgetStop { scope: String, spent: f64, limit: f64 },
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
            Self::AllUnavailable { .. } => ErrorClass::Overloaded,
            _ => ErrorClass::Unknown,
        }
    }
}

/// Where a request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteTarget {
    /// One model, no fallback (the M2 behaviour; a chain of one).
    Pinned { credential_id: String, model: String },
    /// A `[profiles.<name>]` chain.
    Profile(String),
}

pub struct RouteRequest {
    pub session: SessionId,
    pub turn: TurnId,
    pub target: RouteTarget,
    pub transcript: Vec<Turn>,
    pub system: String,
    pub max_tokens: u32,
    pub reasoning: bool,
    pub tools: Vec<ToolSpec>,
    /// Targets that already failed mid-stream for this step; skipped (profiles only).
    pub exclude: Vec<TargetKey>,
}

/// A live response. `served` names who is answering; events end with `Done` or an error.
pub struct RouteStream {
    pub served: ServedBy,
    /// Family/scope to tag reasoning parts with.
    pub family: String,
    pub credential_scope: String,
    pub key: TargetKey,
    pub events: BoxStream<'static, Result<StreamEvent, ProviderError>>,
}

/// Errors after which the agent should discard the partial turn and ask the router again.
pub fn handoff_on(class: &ErrorClass) -> bool {
    !matches!(
        class,
        ErrorClass::Cancelled | ErrorClass::BadRequest { .. } | ErrorClass::ContentPolicy
    )
}

type EventSink = Arc<dyn Fn(AppEvent) + Send + Sync>;

struct Sticky {
    key: TargetKey,
    remaining: u32,
}

struct Inner {
    store: Store,
    secrets: Secrets,
    adapters: HashMap<WireFormat, Arc<dyn ProviderAdapter>>,
    clock: Arc<dyn Clock>,
    health: HealthRegistry,
    profiles: RwLock<BTreeMap<String, Profile>>,
    catalog: RwLock<Catalog>,
    /// Last rate-limit headers per target, with when we saw them.
    limits: Mutex<HashMap<TargetKey, (RateLimitSnapshot, Timestamp)>>,
    sticky: Mutex<HashMap<SessionId, Sticky>>,
    warned: Mutex<HashSet<(SessionId, String)>>,
    events: RwLock<Option<EventSink>>,
}

#[derive(Clone)]
pub struct Router(Arc<Inner>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Router {
    pub fn new(store: Store, secrets: Secrets) -> Self {
        Self::with_clock(store, secrets, Arc::new(SystemClock))
    }

    pub fn with_clock(store: Store, secrets: Secrets, clock: Arc<dyn Clock>) -> Self {
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
        let health = HealthRegistry::default();
        // Quota/auth blocks survive restarts.
        if let Ok(blocks) = store.reader().and_then(|c| termflow_store::route::load_blocks(&c)) {
            for (cred, until, reason) in blocks {
                health.block_credential(&cred, CredentialBlock { until, reason });
            }
        }
        Self(Arc::new(Inner {
            store,
            secrets,
            adapters,
            clock,
            health,
            profiles: RwLock::default(),
            catalog: RwLock::default(),
            limits: Mutex::default(),
            sticky: Mutex::default(),
            warned: Mutex::default(),
            events: RwLock::default(),
        }))
    }

    pub fn set_profiles(&self, p: BTreeMap<String, Profile>) {
        if let Ok(mut g) = self.0.profiles.write() {
            *g = p;
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.0
            .profiles
            .read()
            .map(|g| g.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub fn set_catalog(&self, c: Catalog) {
        if let Ok(mut g) = self.0.catalog.write() {
            *g = c;
        }
    }

    /// Load the catalog from the SQLite cache, then refresh it from models.dev when allowed,
    /// at least one provider is configured (no requests otherwise, §10) and the cache is more
    /// than a day old. Failures keep the cached copy.
    pub async fn load_catalog(&self, overrides: &BTreeMap<String, termflow_catalog::Override>, allow_fetch: bool) {
        let source = termflow_catalog::MODELS_DEV;
        let cached = self
            .0
            .store
            .reader()
            .ok()
            .and_then(|c| termflow_store::route::catalog_get(&c, source).ok().flatten());
        let install = |body: &str| match Catalog::from_models_dev(body) {
            Ok(mut cat) => {
                cat.apply_overrides(overrides);
                tracing::info!(models = cat.len(), "model catalog loaded");
                self.set_catalog(cat);
                true
            }
            Err(e) => {
                tracing::warn!(error = %e, "model catalog unreadable");
                false
            }
        };
        let fresh = cached.as_ref().is_some_and(|(body, at)| {
            install(body) && self.0.clock.now().duration_since(*at) < jiff::SignedDuration::from_hours(24)
        });
        if cached.is_none() {
            // Overrides alone still describe the user's models.
            let mut cat = Catalog::default();
            cat.apply_overrides(overrides);
            self.set_catalog(cat);
        }
        let has_provider = self.credentials().is_ok_and(|c| !c.is_empty());
        if fresh || !allow_fetch || !has_provider {
            return;
        }
        let res = termflow_providers::http_client()
            .get(termflow_catalog::MODELS_DEV_URL)
            .timeout(Duration::from_secs(30))
            .send()
            .await;
        let body = match res {
            Ok(r) if r.status().is_success() => r.text().await.ok(),
            Ok(r) => {
                tracing::warn!(status = %r.status(), "model catalog fetch failed");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, "model catalog fetch failed");
                None
            }
        };
        if let Some(body) = body
            && install(&body)
        {
            let now = self.0.clock.now();
            if let Err(e) = self
                .0
                .store
                .write(move |c| termflow_store::route::catalog_put(c, source, &body, now))
                .await
            {
                tracing::warn!(error = %e, "caching the model catalog failed");
            }
        }
    }

    pub fn set_event_sink(&self, f: EventSink) {
        if let Ok(mut g) = self.0.events.write() {
            *g = Some(f);
        }
    }

    pub fn health(&self) -> &HealthRegistry {
        &self.0.health
    }

    /// The user saved a new key: lift any quota/auth block on it.
    pub async fn clear_credential_block(&self, cred: &str) {
        self.0.health.clear_credential(cred);
        let c = cred.to_owned();
        if let Err(e) = self
            .0
            .store
            .write(move |conn| termflow_store::route::delete_block(conn, &c))
            .await
        {
            tracing::warn!(error = %e, "clearing credential block failed");
        }
    }

    fn emit(&self, ev: AppEvent) {
        if let Some(f) = self.0.events.read().ok().and_then(|g| g.clone()) {
            f(ev);
        }
    }

    fn credentials(&self) -> Result<Vec<Credential>, RouteError> {
        Ok(termflow_store::agent::list_credentials(&self.0.store.reader()?)?)
    }

    fn credential(&self, id: &str) -> Result<Credential, RouteError> {
        termflow_store::agent::get_credential(&self.0.store.reader()?, id)?
            .ok_or_else(|| RouteError::NoCredential(id.into()))
    }

    fn target(&self, cred: &Credential, model: &str) -> Result<Target, RouteError> {
        let api_key = if cred.has_secret {
            Some(self.0.secrets.get(&cred.id)?.ok_or(RouteError::MissingSecret)?)
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
        // Every WireFormat is registered in `with_clock`.
        self.0
            .adapters
            .get(&w)
            .cloned()
            .unwrap_or_else(|| termflow_providers::adapter_for(w, &termflow_providers::http_client()))
    }

    pub async fn list_models(&self, credential_id: &str) -> Result<Vec<ModelInfo>, RouteError> {
        let cred = self.credential(credential_id)?;
        let target = self.target(&cred, "")?;
        Ok(self.adapter(cred.wire).list_models(&target).await?)
    }

    /// Send one request, retrying and falling back as the target allows. When every
    /// candidate of a profile is down, waits for the earliest reset and tries again.
    pub async fn stream(&self, req: RouteRequest, cancel: CancellationToken) -> Result<RouteStream, RouteError> {
        let mut pauses = 0;
        loop {
            match self.route_once(&req, &cancel).await {
                Err(RouteError::AllUnavailable { candidates, resume_at }) => {
                    self.emit(AppEvent::RoutePaused {
                        session_id: req.session,
                        resume_at: resume_at.filter(|_| pauses < MAX_PAUSES),
                        candidates: candidates.clone(),
                    });
                    let Some(at) = resume_at.filter(|_| pauses < MAX_PAUSES) else {
                        return Err(RouteError::AllUnavailable { candidates, resume_at });
                    };
                    pauses += 1;
                    let wait = until(self.0.clock.now(), at);
                    tokio::select! {
                        () = self.0.clock.sleep(wait) => {}
                        () = cancel.cancelled() => return Err(cancelled()),
                    }
                }
                r => return r,
            }
        }
    }

    // One pass over the candidates: the §7.6 selection algorithm and action table.
    #[allow(clippy::too_many_lines)]
    async fn route_once(&self, req: &RouteRequest, cancel: &CancellationToken) -> Result<RouteStream, RouteError> {
        let now = self.0.clock.now();
        let (profile, chain, overflow, content) = match &req.target {
            RouteTarget::Pinned { credential_id, model } => {
                let c = Candidate::new(self.credential(credential_id)?, model);
                (None, vec![c], vec![], vec![])
            }
            RouteTarget::Profile(name) => {
                let p = self
                    .0
                    .profiles
                    .read()
                    .ok()
                    .and_then(|g| g.get(name).cloned())
                    .ok_or_else(|| RouteError::NoProfile(name.clone()))?;
                let creds = self.credentials()?;
                let chain = select::resolve_all(&p.chain, &creds);
                let overflow = select::resolve_all(&p.on_context_overflow, &creds);
                let content = select::resolve_all(&p.on_content_policy, &creds);
                (Some(p), chain, overflow, content)
            }
        };
        if let Some(p) = &profile {
            self.check_budget(req.session, p, now)?;
        }
        let policy = profile.as_ref().map_or_else(RetryPolicy::default, |p| RetryPolicy {
            transient_max_wait: Duration::from_secs(p.retry.transient_max_wait_s),
            overloaded_retries: p.retry.overloaded_retries,
            ..RetryPolicy::default()
        });
        let primary = chain.first().map(|c| c.key.clone());
        let primary_label = chain.first().map(Candidate::label).unwrap_or_default();
        let est = estimate::estimate(&req.transcript, &req.system, &req.tools);

        // Order: hysteresis sticky first, then the chain; unhealthy, excluded and incapable
        // candidates drop out (remembered for the paused panel). Candidates whose rate-limit
        // headers say the request won't fit go last; candidates too small go after overflow.
        let mut states: Vec<CandidateState> = Vec::new();
        let mut queue: VecDeque<Candidate> = VecDeque::new();
        let mut limited: Vec<Candidate> = Vec::new();
        let mut small: Vec<Candidate> = Vec::new();
        let mut ordered = chain.clone();
        if let Some(p) = &profile
            && let Some(s) = lock(&self.0.sticky).get(&req.session)
            && (s.remaining > 0 || p.switch_back == SwitchBack::Never)
            && let Some(i) = ordered.iter().position(|c| c.key == s.key)
        {
            let c = ordered.remove(i);
            ordered.insert(0, c);
        }
        let catalog = self.0.catalog.read().map(|g| g.clone()).unwrap_or_default();
        let require = profile.as_ref().map(|p| p.require.clone()).unwrap_or_default();
        let pinned = profile.is_none();
        for c in ordered.into_iter().chain(overflow.iter().cloned()) {
            if queue.iter().chain(&limited).chain(&small).any(|q| q.key == c.key) {
                continue;
            }
            let caps = catalog.get(&c.key.provider, &c.model);
            let state = self.0.health.state(&c.key, now);
            if !pinned && req.exclude.contains(&c.key) {
                states.push(cand_state(&c, "failed", None));
            } else if let HealthState::Open { until, reason } = state {
                if pinned {
                    // A pinned model is always tried; the provider's answer is the truth.
                    queue.push_back(c);
                } else {
                    states.push(cand_state(&c, &reason, until));
                }
            } else if !pinned && !select::capable(caps, &require) {
                states.push(cand_state(&c, "unfit", None));
            } else if !select::fits(caps, est, req.max_tokens) {
                small.push(c);
            } else if self.rate_limited(&c.key, est, now) {
                limited.push(c);
            } else {
                queue.push_back(c);
            }
        }
        let rate_switch = !limited.is_empty() && !queue.is_empty();
        queue.extend(limited);
        let mut transcript = req.transcript.clone();
        let mut trimmed = false;
        if queue.is_empty() && !small.is_empty() {
            // Nothing fits as is: trim old tool output for the first candidate (§7.7 rule 4).
            let c = small.remove(0);
            let ctx = catalog
                .get(&c.key.provider, &c.model)
                .and_then(|m| m.context)
                .unwrap_or(u32::MAX);
            trimmed = true;
            estimate::trim_tool_results(
                &mut transcript,
                &req.system,
                &req.tools,
                ctx.saturating_sub(req.max_tokens.min(ctx / 4)),
                KEEP_LAST_TURNS,
            );
            queue.push_back(c);
        }
        for c in small {
            states.push(cand_state(&c, "too_small", None));
        }

        let mut tried: HashSet<TargetKey> = HashSet::new();
        let mut last: Option<(ProviderError, String)> = None;
        let mut reason = if rate_switch {
            "rate_limit_headers".to_owned()
        } else {
            String::new()
        };
        let mut left_until: Option<Timestamp> = None;
        while let Some(c) = queue.pop_front() {
            if !tried.insert(c.key.clone()) {
                continue;
            }
            let mut retries = 0;
            let mut stripped = false;
            loop {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                let e = match self
                    .attempt(req, &c, &transcript, profile.as_ref(), stripped, retries, cancel)
                    .await
                {
                    Ok(s) => {
                        self.on_success(req.session, &c, primary.as_ref(), profile.as_ref());
                        if Some(&c.key) != primary.as_ref() && !pinned {
                            self.emit(AppEvent::RouteSwitched {
                                session_id: req.session,
                                from: primary_label.clone(),
                                to: c.label(),
                                reason: if reason.is_empty() {
                                    "unavailable".into()
                                } else {
                                    reason.clone()
                                },
                                until: left_until,
                            });
                        }
                        return Ok(s);
                    }
                    Err(RouteError::Provider(e)) => e,
                    Err(other) => {
                        // Missing key / credential: skip this candidate, keep going.
                        tracing::warn!(target = %c.label(), error = %other, "candidate unusable");
                        states.push(cand_state(&c, "unusable", None));
                        if pinned {
                            return Err(other);
                        }
                        break;
                    }
                };
                let now = self.0.clock.now();
                if counts_for_health(&e.class) {
                    self.0.health.failure(&c.key, now, e.class.name());
                }
                if Some(&c.key) == primary.as_ref() || reason.is_empty() {
                    reason = e.class.name().into();
                }
                match decide(&e.class, retries, stripped, &policy, now, policy::jitter()) {
                    Action::Retry { after } => {
                        if after >= Duration::from_secs(1) {
                            self.emit(AppEvent::RouteWaiting {
                                session_id: req.session,
                                target: c.label(),
                                seconds: u32::try_from(after.as_secs()).unwrap_or(u32::MAX),
                            });
                        }
                        tokio::select! {
                            () = self.0.clock.sleep(after) => {}
                            () = cancel.cancelled() => return Err(cancelled()),
                        }
                        retries += 1;
                    }
                    Action::Next(cool) => {
                        let until = self.apply_cool(&c, &cool, e.class.name()).await;
                        if Some(&c.key) == primary.as_ref() {
                            left_until = until;
                        }
                        states.push(cand_state(&c, e.class.name(), until));
                        last = Some((e, c.label()));
                        break;
                    }
                    Action::Overflow => {
                        let fresh: Vec<Candidate> = overflow
                            .iter()
                            .filter(|o| !tried.contains(&o.key) && self.0.health.available(&o.key, now))
                            .cloned()
                            .collect();
                        if fresh.is_empty() && !trimmed {
                            trimmed = true;
                            let budget = estimate::estimate(&transcript, &req.system, &req.tools) / 2;
                            estimate::trim_tool_results(
                                &mut transcript,
                                &req.system,
                                &req.tools,
                                budget,
                                KEEP_LAST_TURNS,
                            );
                            continue;
                        }
                        for o in fresh.into_iter().rev() {
                            queue.push_front(o);
                        }
                        states.push(cand_state(&c, "context_overflow", None));
                        last = Some((e, c.label()));
                        break;
                    }
                    Action::ContentPolicy => {
                        for o in content.iter().rev().filter(|o| !tried.contains(&o.key)) {
                            queue.push_front(o.clone());
                        }
                        states.push(cand_state(&c, "content_policy", None));
                        last = Some((e, c.label()));
                        break;
                    }
                    Action::StripNative => stripped = true,
                    Action::Surface => return Err(e.into()),
                }
            }
        }
        if pinned {
            return Err(last.map_or_else(
                || RouteError::NoCredential(String::new()),
                |(e, _)| RouteError::Provider(e),
            ));
        }
        let keys: Vec<TargetKey> = chain.iter().map(|c| c.key.clone()).collect();
        let now = self.0.clock.now();
        Err(RouteError::AllUnavailable {
            candidates: states,
            resume_at: self.0.health.earliest_reset(&keys, now).filter(|t| *t > now),
        })
    }

    fn rate_limited(&self, k: &TargetKey, est: u32, now: Timestamp) -> bool {
        lock(&self.0.limits).get(k).is_some_and(|(rl, seen)| {
            let remaining = rl.input_tokens_remaining.or(rl.tokens_remaining);
            // Without a reset time, headers count for a minute (windows are typically that short).
            let fresh = rl
                .reset_at
                .map_or_else(|| now < clock::after(*seen, RATE_LIMIT_TTL), |r| r > now);
            fresh && (remaining.is_some_and(|r| r < u64::from(est)) || rl.requests_remaining == Some(0))
        })
    }

    fn on_success(&self, session: SessionId, c: &Candidate, primary: Option<&TargetKey>, p: Option<&Profile>) {
        self.0.health.success(&c.key, self.0.clock.now());
        let Some(p) = p else { return };
        let mut sticky = lock(&self.0.sticky);
        if Some(&c.key) == primary {
            sticky.remove(&session);
            return;
        }
        match sticky.get_mut(&session) {
            Some(s) if s.key == c.key => s.remaining = s.remaining.saturating_sub(1),
            _ => {
                sticky.insert(
                    session,
                    Sticky {
                        key: c.key.clone(),
                        remaining: p.hysteresis_turns,
                    },
                );
            }
        }
    }

    async fn apply_cool(&self, c: &Candidate, cool: &Cool, reason: &str) -> Option<Timestamp> {
        apply_cool(&self.0, c, cool, reason).await
    }

    fn check_budget(&self, session: SessionId, p: &Profile, now: Timestamp) -> Result<(), RouteError> {
        let b = &p.budget;
        let conn = self.0.store.reader()?;
        let utc = jiff::tz::TimeZone::UTC;
        let date = now.to_zoned_datetime(utc.clone()).date();
        let at = |d: jiff::civil::Date| d.to_zoned_datetime(utc.clone()).map_or(now, |z| z.timestamp());
        let (day, month) = (at(date), at(date.first_of_month()));
        let scopes = [
            ("session", b.session_usd, Timestamp::UNIX_EPOCH, Some(session)),
            ("day", b.day_usd, day, None),
            ("month", b.month_usd, month, None),
        ];
        for (scope, limit, since, sess) in scopes {
            let Some(limit) = limit else { continue };
            let spent = termflow_store::route::spend_since(&conn, since, sess, None)?;
            if spent >= limit && b.hard_stop {
                self.emit(AppEvent::BudgetStopped {
                    session_id: session,
                    scope: scope.into(),
                    spent_usd: spent,
                    limit_usd: limit,
                });
                return Err(RouteError::BudgetStop {
                    scope: scope.into(),
                    spent,
                    limit,
                });
            }
            if spent >= limit * b.warn_at && lock(&self.0.warned).insert((session, scope.into())) {
                self.emit(AppEvent::BudgetWarning {
                    session_id: session,
                    scope: scope.into(),
                    spent_usd: spent,
                    limit_usd: limit,
                });
            }
        }
        Ok(())
    }

    /// One HTTP attempt. Returns once the first content event (or `Done`) arrived; an error
    /// before that comes back as `Err` so the caller can retry or move on.
    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        &self,
        req: &RouteRequest,
        c: &Candidate,
        transcript: &[Turn],
        profile: Option<&Profile>,
        stripped: bool,
        retry_no: u32,
        cancel: &CancellationToken,
    ) -> Result<RouteStream, RouteError> {
        let target = self.target(&c.cred, &c.model)?;
        let adapter = self.adapter(c.cred.wire);
        let family = c.cred.kind.id().to_owned();
        let mut system = req.system.clone();
        if profile.is_some_and(|p| p.handoff_note) && last_family(transcript).is_some_and(|f| f != family) {
            system.push_str(HANDOFF_NOTE);
        }
        let ctx = RenderContext {
            model: c.model.clone(),
            system,
            max_tokens: req.max_tokens,
            reasoning: req.reasoning,
            // A different family tag makes every stored reasoning block ineligible to re-send.
            family: if stripped {
                format!("{family}#stripped")
            } else {
                family.clone()
            },
            credential_scope: c.cred.id.clone(),
            tools: req.tools.clone(),
            native_ids: termflow_store::agent::native_ids(&self.0.store.reader()?, req.session, &c.cred.id)
                .unwrap_or_default(),
        };
        let body = adapter.render(transcript, &ctx)?;

        let attempt = AttemptId::new();
        {
            let (p, m, cr) = (c.key.provider.clone(), c.model.clone(), c.cred.id.clone());
            let (session, turn) = (req.session, req.turn);
            self.0
                .store
                .write(move |conn| {
                    termflow_store::agent::start_attempt(
                        conn,
                        &AttemptStart {
                            id: attempt,
                            session,
                            turn,
                            provider: &p,
                            model: &m,
                            credential_id: Some(&cr),
                            retry_no,
                        },
                    )
                })
                .await?;
        }
        let mut track = Track {
            inner: self.0.clone(),
            attempt,
            started: Instant::now(),
            cand: c.clone(),
            usage: Usage::default(),
            ttft: None,
            served_model: None,
        };
        let mut inner = match adapter.stream(&target, body, cancel.clone()).await {
            Ok(s) => s,
            Err(e) => {
                track.finish(e.status, Some(&e.class)).await;
                return Err(e.into());
            }
        };
        // Hold events back until real content arrives, so early failures stay retryable.
        let mut head = Vec::new();
        loop {
            match inner.next().await {
                None => break,
                Some(Ok(ev)) => {
                    let content = track.see(&ev);
                    head.push(Ok(ev));
                    if content {
                        break;
                    }
                }
                Some(Err(e)) => {
                    track.finish(Some(200), Some(&e.class)).await;
                    return Err(e.into());
                }
            }
        }
        let served = ServedBy {
            provider: c.key.provider.clone(),
            model: c.model.clone(),
            credential_id: Some(c.cred.id.clone()),
            attempt_id: attempt,
        };
        let (tx, rx) = mpsc::channel(256);
        tokio::spawn(forward(head, inner, tx, track));
        Ok(RouteStream {
            served,
            family,
            credential_scope: c.cred.id.clone(),
            key: c.key.clone(),
            events: tokio_stream::wrappers::ReceiverStream::new(rx).boxed(),
        })
    }
}

/// Bookkeeping for one attempt row: usage, TTFT, served model, rate-limit headers.
struct Track {
    inner: Arc<Inner>,
    attempt: AttemptId,
    started: Instant,
    cand: Candidate,
    usage: Usage,
    ttft: Option<u64>,
    served_model: Option<String>,
}

impl Track {
    /// Record what an event tells us. Returns whether it is real content.
    fn see(&mut self, ev: &StreamEvent) -> bool {
        match ev {
            StreamEvent::TextDelta(_)
            | StreamEvent::ReasoningDelta(_)
            | StreamEvent::ReasoningDone { .. }
            | StreamEvent::ToolCallStart { .. } => {
                self.ttft
                    .get_or_insert_with(|| u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX));
                true
            }
            StreamEvent::Done { .. } => true,
            StreamEvent::Usage(u) => {
                self.usage.merge(*u);
                false
            }
            StreamEvent::ServedModel(m) => {
                self.served_model = Some(m.clone());
                false
            }
            StreamEvent::RateLimit(rl) => {
                let now = self.inner.clock.now();
                lock(&self.inner.limits).insert(self.cand.key.clone(), (rl.clone(), now));
                false
            }
            StreamEvent::ToolCallArgsDelta { .. } | StreamEvent::ToolCallEnd { .. } => false,
        }
    }

    /// Close the attempt row with cost from catalog prices of the served model.
    /// Logging only: never fails the request.
    async fn finish(&self, status: Option<u16>, error: Option<&ErrorClass>) {
        let model = self.served_model.clone().unwrap_or_else(|| self.cand.model.clone());
        let cost = self.inner.catalog.read().ok().and_then(|cat| {
            cat.get(&self.cand.key.provider, &model)
                .and_then(|m| m.price)
                .map(|p| termflow_catalog::cost(&p, &self.usage, self.cand.cred.wire != WireFormat::AnthropicMessages))
        });
        let (id, usage, ttft, served, err) = (
            self.attempt,
            self.usage,
            self.ttft,
            self.served_model.clone(),
            error.cloned(),
        );
        let res = self
            .inner
            .store
            .write(move |c| {
                termflow_store::agent::finish_attempt(
                    c,
                    id,
                    &AttemptEnd {
                        http_status: status,
                        error: err.as_ref(),
                        ttft_ms: ttft,
                        usage,
                        served_model: served.as_deref(),
                        cost_usd: cost,
                    },
                )
            })
            .await;
        if let Err(e) = res {
            tracing::warn!(error = %e, "recording attempt failed");
        }
    }
}

async fn forward(
    head: Vec<Result<StreamEvent, ProviderError>>,
    mut inner: EventStream,
    tx: mpsc::Sender<Result<StreamEvent, ProviderError>>,
    mut track: Track,
) {
    let mut error: Option<ErrorClass> = None;
    for ev in head {
        if tx.send(ev).await.is_err() {
            track.finish(Some(200), Some(&ErrorClass::Cancelled)).await;
            return;
        }
    }
    while let Some(ev) = inner.next().await {
        match &ev {
            Ok(e) => {
                track.see(e);
            }
            Err(e) => error = Some(e.class.clone()),
        }
        if tx.send(ev).await.is_err() {
            error.get_or_insert(ErrorClass::Cancelled);
            break;
        }
    }
    track.finish(Some(200), error.as_ref()).await;
    // A mid-stream failure counts against the target like any other (the agent hands off).
    let now = track.inner.clock.now();
    match error {
        Some(class) => {
            if counts_for_health(&class) {
                track.inner.health.failure(&track.cand.key, now, class.name());
            }
            if let Action::Next(cool) = decide(&class, u32::MAX, true, &RetryPolicy::default(), now, 0.0) {
                apply_cool(&track.inner, &track.cand, &cool, class.name()).await;
            }
        }
        None => track.inner.health.success(&track.cand.key, now),
    }
}

/// Take a candidate out of rotation per the action table; credential blocks are persisted.
async fn apply_cool(inner: &Inner, c: &Candidate, cool: &Cool, reason: &str) -> Option<Timestamp> {
    match cool {
        Cool::None => None,
        Cool::Target(t) => {
            inner.health.cool_target(&c.key, *t, reason);
            Some(*t)
        }
        Cool::Credential(until) => {
            inner.health.block_credential(
                &c.cred.id,
                CredentialBlock {
                    until: *until,
                    reason: reason.into(),
                },
            );
            let (id, u, r) = (c.cred.id.clone(), *until, reason.to_owned());
            if let Err(e) = inner
                .store
                .write(move |conn| termflow_store::route::put_block(conn, &id, u, &r))
                .await
            {
                tracing::warn!(error = %e, "persisting credential block failed");
            }
            *until
        }
    }
}

/// Availability failures feed the circuit breaker; request-specific ones don't.
fn counts_for_health(class: &ErrorClass) -> bool {
    matches!(
        class,
        ErrorClass::Overloaded | ErrorClass::Unknown | ErrorClass::TransientRateLimit { .. }
    )
}

fn cand_state(c: &Candidate, state: &str, until: Option<Timestamp>) -> CandidateState {
    CandidateState {
        target: c.label(),
        state: state.into(),
        until,
    }
}

/// Provider family of the most recent assistant turn.
fn last_family(transcript: &[Turn]) -> Option<&str> {
    transcript
        .iter()
        .rev()
        .find(|t| t.role == Role::Assistant)
        .and_then(|t| t.served_by.as_ref())
        .map(|s| s.provider.as_str())
}

fn cancelled() -> RouteError {
    RouteError::Provider(ProviderError::new(ErrorClass::Cancelled, None, "cancelled"))
}
