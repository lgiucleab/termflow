#![allow(clippy::unwrap_used, clippy::float_cmp, clippy::unused_self, clippy::assert_is_empty)] // test code

//! `BUILD_PROMPT` §12 M3 acceptance against two mock providers and a fake clock.
//! (c) and (f) involve the agent loop and live in `termflow-agent`.

use std::collections::HashMap;
use std::sync::Arc;

use mock_provider::{MockProvider, Script};
use termflow_catalog::{Override, Price};
use termflow_config::{BudgetConfig, Profile};
use termflow_secrets::{MemoryStore, Redactor};
use termflow_types::{Part, ProviderKind, Role, TurnStatus};

use super::*;
use crate::clock::FakeClock;

const ANTHROPIC_OK: &str = include_str!("../../../tests/fixtures/http/anthropic/text_thinking_tool.sse");
const CHAT_OK: &str = include_str!("../../../tests/fixtures/http/openai_chat/text_tool.sse");
const SPEND_LIMIT: &str = include_str!("../../../tests/fixtures/http/anthropic/429_spend_limit.json");
const MID_ERROR: &str = include_str!("../../../tests/fixtures/http/anthropic/error_after_tokens.sse");
const CATALOG: &str = include_str!("../../../tests/fixtures/catalog/models_dev.json");

const PRIMARY: &str = "anthropic/claude-test-model@primary";
const BACKUP: &str = "openrouter/backup-model@backup";

struct H {
    _d: tempfile::TempDir,
    store: Store,
    router: Router,
    clock: Arc<FakeClock>,
    a: MockProvider,
    b: MockProvider,
    events: Arc<Mutex<Vec<AppEvent>>>,
}

fn overloaded() -> Script {
    Script::json(
        529,
        &serde_json::json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
    )
    .with_status(529)
}

fn profile(chain: &[&str]) -> Profile {
    Profile {
        chain: chain.iter().map(|s| (*s).to_owned()).collect(),
        ..Profile::default()
    }
}

async fn harness(p: Profile) -> H {
    let a = MockProvider::start().await.unwrap();
    let b = MockProvider::start().await.unwrap();
    a.set_default(Script::sse(ANTHROPIC_OK));
    b.set_default(Script::sse(CHAT_OK));
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("t.db")).unwrap();
    let secrets = Secrets::new(Arc::new(MemoryStore::default()), Redactor::new());
    for (id, kind, wire, url, label) in [
        (
            "c1",
            ProviderKind::Anthropic,
            WireFormat::AnthropicMessages,
            a.url(),
            "primary",
        ),
        (
            "c2",
            ProviderKind::Openrouter,
            WireFormat::OpenaiChat,
            b.url(),
            "backup",
        ),
    ] {
        secrets.set(id, "sk-test-0000000000").unwrap();
        let cred = Credential {
            id: id.into(),
            kind,
            label: label.into(),
            base_url: url,
            wire,
            has_secret: true,
            created_at: jiff::Timestamp::now(),
        };
        store
            .write(move |c| termflow_store::agent::insert_credential(c, &cred))
            .await
            .unwrap();
    }
    let clock = Arc::new(FakeClock::new(jiff::Timestamp::now()));
    let router = Router::with_clock(store.clone(), secrets, clock.clone());
    router.set_profiles([("main".to_owned(), p)].into_iter().collect());
    let mut cat = Catalog::from_models_dev(CATALOG).unwrap();
    // The fixture's OpenRouter stream reports `served-test-model`; it is priced, the
    // requested `backup-model` is not, so a non-zero cost proves served-model pricing.
    cat.apply_overrides(&HashMap::from([(
        "openrouter/served-test-model".to_owned(),
        Override {
            price: Some(Price {
                input: 1.0,
                output: 2.0,
                cache_read: None,
                cache_write: None,
            }),
            ..Default::default()
        },
    )]));
    router.set_catalog(cat);
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    router.set_event_sink(Arc::new(move |e| lock(&sink).push(e)));
    H {
        _d: d,
        store,
        router,
        clock,
        a,
        b,
        events,
    }
}

impl H {
    async fn session(&self) -> SessionId {
        let s = SessionId::new();
        self.store
            .write(move |c| termflow_store::agent::create_session(c, s, None, None, None))
            .await
            .unwrap();
        s
    }

    fn req(&self, session: SessionId, target: RouteTarget) -> RouteRequest {
        RouteRequest {
            session,
            turn: TurnId::new(),
            target,
            transcript: vec![Turn::new(
                session,
                Role::User,
                vec![Part::Text { text: "hi".into() }],
                TurnStatus::Committed,
            )],
            system: String::new(),
            max_tokens: 1000,
            reasoning: true,
            tools: vec![],
            exclude: vec![],
        }
    }

    async fn run(&self, session: SessionId) -> Result<(RouteStream, Vec<StreamEvent>), RouteError> {
        let mut s = self
            .router
            .stream(
                self.req(session, RouteTarget::Profile("main".into())),
                CancellationToken::new(),
            )
            .await?;
        let mut evs = Vec::new();
        while let Some(e) = s.events.next().await {
            evs.push(e.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(50)).await; // attempt rows close after the stream
        Ok((s, evs))
    }

    fn events(&self) -> Vec<AppEvent> {
        lock(&self.events).clone()
    }

    fn attempts(&self) -> Vec<(String, Option<String>, Option<f64>, i64)> {
        let c = self.store.reader().unwrap();
        let mut st = c
            .prepare("SELECT model, error_class, cost_usd, retry_no FROM attempts ORDER BY started_at, rowid")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

fn first_of_next_month() -> Timestamp {
    let utc = jiff::tz::TimeZone::UTC;
    let d = Timestamp::now().to_zoned_datetime(utc.clone()).date().first_of_month();
    let next = d.checked_add(jiff::ToSpan::month(1)).unwrap();
    next.to_zoned_datetime(utc).unwrap().timestamp()
}

// (a)
#[tokio::test]
async fn spend_limit_switches_at_once_and_blocks_until_next_month() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    h.a.push(Script::json(429, &serde_json::from_str(SPEND_LIMIT).unwrap()).with_status(429));
    let (s, _) = h.run(h.session().await).await.unwrap();
    assert_eq!(s.served.credential_id.as_deref(), Some("c2"));
    assert_eq!(h.a.requests().len(), 1, "quota is never retried");
    assert!(h.clock.slept().is_empty());
    let k = TargetKey {
        provider: "anthropic".into(),
        model: "claude-test-model".into(),
        credential: Some("c1".into()),
    };
    let HealthState::Open { until, .. } = h.router.health().state(&k, h.clock.now()) else {
        panic!("primary should be out of rotation")
    };
    assert_eq!(until, Some(first_of_next_month()));
    // Persisted: a restart keeps the block.
    let blocks = termflow_store::route::load_blocks(&h.store.reader().unwrap()).unwrap();
    assert_eq!(
        blocks,
        vec![("c1".into(), Some(first_of_next_month()), "quota_exhausted".into())]
    );
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, AppEvent::RouteSwitched { reason, until, .. }
        if reason == "quota_exhausted" && *until == Some(first_of_next_month())))
    );
}

// (b) + (e)
#[tokio::test]
async fn overloaded_retries_twice_with_backoff_then_switches_and_every_attempt_is_priced() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    for _ in 0..3 {
        h.a.push(overloaded());
    }
    let (s, _) = h.run(h.session().await).await.unwrap();
    assert_eq!(s.served.credential_id.as_deref(), Some("c2"));
    assert_eq!(h.a.requests().len(), 3);
    let slept = h.clock.slept();
    assert_eq!(slept.len(), 2);
    assert!((Duration::from_secs(1)..Duration::from_millis(1500)).contains(&slept[0]));
    assert!((Duration::from_secs(2)..Duration::from_millis(2500)).contains(&slept[1]));
    let rows = h.attempts();
    assert_eq!(rows.len(), 4);
    for (i, (model, err, cost, retry)) in rows.iter().take(3).enumerate() {
        assert_eq!(
            (model.as_str(), err.as_deref(), *cost, *retry),
            (
                "claude-test-model",
                Some("overloaded"),
                Some(0.0),
                i64::try_from(i).unwrap()
            )
        );
    }
    let (model, err, cost, _) = &rows[3];
    assert_eq!((model.as_str(), err.as_deref()), ("served-test-model", None));
    assert!(cost.unwrap() > 0.0, "priced from the served model");
}

// (g)
#[tokio::test]
async fn short_retry_after_waits_and_retries_the_same_target() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    h.a.push(
        Script::json(
            429,
            &serde_json::json!({"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}),
        )
        .with_status(429)
        .with_header("retry-after", "3"),
    );
    let (s, _) = h.run(h.session().await).await.unwrap();
    assert_eq!(s.served.credential_id.as_deref(), Some("c1"));
    assert_eq!(h.clock.slept(), vec![Duration::from_secs(3)]);
    assert_eq!(h.a.requests().len(), 2);
    assert!(h.b.requests().is_empty());
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, AppEvent::RouteWaiting { seconds: 3, .. }))
    );
}

// (d)
#[tokio::test]
async fn context_overflow_uses_the_overflow_target() {
    let mut p = profile(&[PRIMARY]);
    p.on_context_overflow = vec![BACKUP.into()];
    let h = harness(p).await;
    h.a.push(
        Script::json(
            413,
            &serde_json::json!({"type":"error","error":{"type":"request_too_large","message":"Request exceeds the maximum size"}}),
        )
        .with_status(413),
    );
    let (s, evs) = h.run(h.session().await).await.unwrap();
    assert_eq!(s.served.credential_id.as_deref(), Some("c2"));
    assert!(matches!(evs.last(), Some(StreamEvent::Done { .. })));
    assert_eq!(h.a.requests().len(), 1);
}

#[tokio::test]
async fn context_overflow_without_target_trims_old_tool_output_and_retries() {
    let h = harness(profile(&[PRIMARY])).await;
    h.a.push(
        Script::json(
            413,
            &serde_json::json!({"type":"error","error":{"type":"request_too_large","message":"too large"}}),
        )
        .with_status(413),
    );
    let session = h.session().await;
    let mut req = h.req(session, RouteTarget::Profile("main".into()));
    let call = termflow_types::CallId::new();
    let mut transcript = vec![
        Turn::new(
            session,
            Role::Assistant,
            vec![Part::ToolCall {
                call_id: call,
                name: "read".into(),
                args: serde_json::json!({"path": "big"}),
            }],
            TurnStatus::Committed,
        ),
        Turn::new(
            session,
            Role::Tool,
            vec![Part::ToolResult {
                call_id: call,
                content: vec![termflow_types::ToolResultContent::Text {
                    text: "BIGOUTPUT".repeat(2000),
                }],
                is_error: false,
            }],
            TurnStatus::Committed,
        ),
    ];
    transcript.extend((0..KEEP_LAST_TURNS).map(|_| req.transcript[0].clone()));
    req.transcript = transcript;
    let mut s = h.router.stream(req, CancellationToken::new()).await.unwrap();
    while s.events.next().await.is_some() {}
    let reqs = h.a.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].body.to_string().contains("BIGOUTPUT"));
    assert!(!reqs[1].body.to_string().contains("BIGOUTPUT"));
}

// (h)
#[tokio::test]
async fn low_remaining_tokens_switch_before_failing() {
    let mut p = profile(&[PRIMARY, BACKUP]);
    p.hysteresis_turns = 0;
    let h = harness(p).await;
    h.a.push(Script::sse(ANTHROPIC_OK).with_header("anthropic-ratelimit-input-tokens-remaining", "1"));
    let session = h.session().await;
    let (first, _) = h.run(session).await.unwrap();
    assert_eq!(first.served.credential_id.as_deref(), Some("c1"));
    let (second, _) = h.run(session).await.unwrap();
    assert_eq!(second.served.credential_id.as_deref(), Some("c2"));
    assert_eq!(h.a.requests().len(), 1, "no failing request was sent");
    let (again, _) = h.run(session).await.unwrap();
    assert_eq!(
        again.served.credential_id.as_deref(),
        Some("c2"),
        "still limited within the minute"
    );
    assert!(h.attempts().iter().all(|(_, err, ..)| err.is_none()));
    assert!(
        h.events()
            .iter()
            .any(|e| matches!(e, AppEvent::RouteSwitched { reason, .. } if reason == "rate_limit_headers"))
    ); // Headers without a reset time expire: a minute later the primary is first again.
    h.clock.advance(Duration::from_secs(61));
    let (third, _) = h.run(session).await.unwrap();
    assert_eq!(third.served.credential_id.as_deref(), Some("c1"));
    assert_eq!(h.a.requests().len(), 2);
}

// (i)
#[tokio::test]
async fn hysteresis_stays_on_the_fallback_then_returns_when_healthy() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    for _ in 0..3 {
        h.a.push(overloaded());
    }
    let session = h.session().await;
    let mut served = Vec::new();
    for i in 0..5 {
        if i == 1 {
            h.clock.advance(Duration::from_secs(120)); // primary's cooldown is long over
        }
        let (s, _) = h.run(session).await.unwrap();
        served.push(s.served.credential_id.unwrap());
    }
    assert_eq!(served, ["c2", "c2", "c2", "c2", "c1"]);
    assert_eq!(h.a.requests().len(), 4, "primary untouched for the hysteresis turns");
}

// (j)
#[tokio::test]
async fn native_thinking_goes_only_to_the_same_family_and_credential() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    let session = h.session().await;
    let req = |t: RouteTarget| {
        let mut r = h.req(session, t);
        r.transcript.push(Turn::new(
            session,
            Role::Assistant,
            vec![
                Part::Reasoning {
                    family: "anthropic".into(),
                    model: "claude-test-model".into(),
                    credential_scope: "c1".into(),
                    native: serde_json::json!({"type":"thinking","thinking":"hmm","signature":"SIG-123"}),
                    summary_text: None,
                },
                Part::Text { text: "answer".into() },
            ],
            TurnStatus::Committed,
        ));
        r.transcript.push(r.transcript[0].clone());
        r
    };
    for (t, mock, same_family) in [
        (
            RouteTarget::Pinned {
                credential_id: "c1".into(),
                model: "claude-test-model".into(),
            },
            &h.a,
            true,
        ),
        (
            RouteTarget::Pinned {
                credential_id: "c2".into(),
                model: "backup-model".into(),
            },
            &h.b,
            false,
        ),
    ] {
        let mut s = h.router.stream(req(t), CancellationToken::new()).await.unwrap();
        while s.events.next().await.is_some() {}
        let body = mock.requests()[0].body.to_string();
        assert_eq!(body.contains("SIG-123"), same_family, "{body}");
        assert!(body.contains("answer"));
    }
}

#[tokio::test]
async fn cross_vendor_request_carries_the_handoff_note() {
    let h = harness(profile(&[BACKUP])).await;
    let session = h.session().await;
    let mut r = h.req(session, RouteTarget::Profile("main".into()));
    let mut prev = Turn::new(
        session,
        Role::Assistant,
        vec![Part::Text { text: "a".into() }],
        TurnStatus::Committed,
    );
    prev.served_by = Some(ServedBy {
        provider: "anthropic".into(),
        model: "m".into(),
        credential_id: Some("c1".into()),
        attempt_id: AttemptId::new(),
    });
    r.transcript.push(prev);
    let mut s = h.router.stream(r, CancellationToken::new()).await.unwrap();
    while s.events.next().await.is_some() {}
    assert!(h.b.requests()[0].body.to_string().contains("another model started"));
}

// (k)
#[tokio::test]
async fn all_down_pauses_then_resumes_at_the_earliest_reset() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    let chat_overloaded =
        Script::json(503, &serde_json::json!({"error":{"message":"overloaded","code":503}})).with_status(503);
    for _ in 0..3 {
        h.a.push(overloaded());
        h.b.push(chat_overloaded.clone());
    }
    let (s, _) = h.run(h.session().await).await.unwrap();
    assert_eq!(
        s.served.credential_id.as_deref(),
        Some("c1"),
        "primary probed after the reset"
    );
    let paused = h
        .events()
        .into_iter()
        .find_map(|e| match e {
            AppEvent::RoutePaused {
                resume_at, candidates, ..
            } => Some((resume_at, candidates)),
            _ => None,
        })
        .unwrap();
    let resume = paused.0.unwrap();
    assert_eq!(paused.1.len(), 2);
    assert!(paused.1.iter().all(|c| c.state == "overloaded"));
    // The last sleep is the pause, and it lasted until the earliest reset.
    assert!(*h.clock.slept().last().unwrap() >= Duration::from_secs(25));
    assert!(h.clock.now() >= resume);
    assert_eq!((h.a.requests().len(), h.b.requests().len()), (4, 3));
}

// (l)
#[tokio::test]
async fn budget_hard_stop_pauses_without_switching() {
    let mut p = profile(&[PRIMARY, BACKUP]);
    p.budget = BudgetConfig {
        session_usd: Some(0.01),
        ..BudgetConfig::default()
    };
    let h = harness(p).await;
    let session = h.session().await;
    h.store
        .write(move |c| {
            let id = AttemptId::new();
            termflow_store::agent::start_attempt(
                c,
                &AttemptStart {
                    id,
                    session,
                    turn: TurnId::new(),
                    provider: "anthropic",
                    model: "claude-test-model",
                    credential_id: Some("c1"),
                    retry_no: 0,
                },
            )?;
            termflow_store::agent::finish_attempt(
                c,
                id,
                &AttemptEnd {
                    http_status: Some(200),
                    error: None,
                    ttft_ms: None,
                    usage: Usage::default(),
                    served_model: None,
                    cost_usd: Some(0.02),
                },
            )
        })
        .await
        .unwrap();
    let Err(RouteError::BudgetStop { scope, .. }) = h.run(session).await else {
        panic!("expected a budget stop")
    };
    assert_eq!(scope, "session");
    assert!(h.a.requests().is_empty() && h.b.requests().is_empty());
    assert!(h.events().iter().any(|e| matches!(e, AppEvent::BudgetStopped { .. })));
}

#[tokio::test]
async fn mid_stream_error_reaches_the_caller_and_counts_against_the_target() {
    let h = harness(profile(&[PRIMARY, BACKUP])).await;
    h.a.push(Script::sse(MID_ERROR));
    let session = h.session().await;
    let mut s = h
        .router
        .stream(
            h.req(session, RouteTarget::Profile("main".into())),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let mut got_text = false;
    let mut err = None;
    while let Some(e) = s.events.next().await {
        match e {
            Ok(StreamEvent::TextDelta(_)) => got_text = true,
            Ok(_) => {}
            Err(e) => err = Some(e.class),
        }
    }
    assert!(got_text);
    assert_eq!(err, Some(ErrorClass::Overloaded));
    assert!(handoff_on(&ErrorClass::Overloaded) && !handoff_on(&ErrorClass::Cancelled));
}

#[tokio::test]
async fn pinned_model_keeps_m2_behaviour_plus_retries() {
    let h = harness(profile(&[PRIMARY])).await;
    for _ in 0..3 {
        h.a.push(overloaded());
    }
    let session = h.session().await;
    let pinned = RouteTarget::Pinned {
        credential_id: "c1".into(),
        model: "claude-test-model".into(),
    };
    let Err(e) = h.router.stream(h.req(session, pinned), CancellationToken::new()).await else {
        panic!()
    };
    assert_eq!(e.class(), ErrorClass::Overloaded);
    assert_eq!(h.a.requests().len(), 3);
    assert!(h.b.requests().is_empty());
    // The key reached the provider from the keychain store, never from the DB.
    assert!(
        h.a.requests()[0]
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-test-0000000000")
    );
}
