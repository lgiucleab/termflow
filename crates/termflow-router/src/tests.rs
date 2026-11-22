#![allow(clippy::unwrap_used)] // test code

use std::sync::Arc;

use mock_provider::{MockProvider, Script};
use termflow_secrets::{MemoryStore, Redactor};
use termflow_types::{ProviderKind, Role, TurnStatus};

use super::*;

const SSE: &str = include_str!("../../../tests/fixtures/http/anthropic/text_thinking_tool.sse");

async fn setup(mock: &MockProvider) -> (tempfile::TempDir, Store, Router) {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("t.db")).unwrap();
    let secrets = Secrets::new(Arc::new(MemoryStore::default()), Redactor::new());
    secrets.set("c1", "sk-ant-test-0000000000").unwrap();
    let cred = Credential {
        id: "c1".into(),
        kind: ProviderKind::Anthropic,
        label: "test".into(),
        base_url: mock.url(),
        wire: WireFormat::AnthropicMessages,
        has_secret: true,
        created_at: jiff::Timestamp::now(),
    };
    store
        .write(move |c| termflow_store::agent::insert_credential(c, &cred))
        .await
        .unwrap();
    let router = Router::new(store.clone(), secrets);
    (d, store, router)
}

async fn req(store: &Store) -> RouteRequest {
    let session = SessionId::new();
    store
        .write(move |c| termflow_store::agent::create_session(c, session, Some("c1"), None))
        .await
        .unwrap();
    RouteRequest {
        session,
        turn: TurnId::new(),
        credential_id: "c1".into(),
        model: "claude-test-model".into(),
        transcript: vec![Turn::new(
            session,
            Role::User,
            vec![termflow_types::Part::Text { text: "hi".into() }],
            TurnStatus::Committed,
        )],
        system: String::new(),
        max_tokens: 100,
        reasoning: true,
        tools: vec![],
    }
}

fn attempt_row(store: &Store) -> (Option<String>, i64, i64, Option<i64>, Option<String>) {
    store
        .reader()
        .unwrap()
        .query_row(
            "SELECT error_class, input_tokens, output_tokens, http_status, model FROM attempts",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
}

#[tokio::test]
async fn success_is_logged_with_usage_and_served_model() {
    let mock = MockProvider::start().await.unwrap();
    mock.push(Script::sse(SSE));
    let (_d, store, router) = setup(&mock).await;
    let mut s = router
        .stream(req(&store).await, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(s.served.provider, "anthropic");
    while s.events.next().await.is_some() {}
    tokio::time::sleep(Duration::from_millis(100)).await; // attempt row is written after the stream ends
    let (err, input, output, status, model) = attempt_row(&store);
    assert_eq!((err, input, output, status), (None, 120, 42, Some(200)));
    assert_eq!(model.as_deref(), Some("claude-test-model"));
    // The key reached the provider from the keychain store, never from the DB.
    assert!(
        mock.requests()[0]
            .headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-ant-test-0000000000")
    );
}

#[tokio::test]
async fn http_error_is_logged_and_returned_classified() {
    let mock = MockProvider::start().await.unwrap();
    mock.push(
        Script::json(
            529,
            &serde_json::json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
        )
        .with_status(529),
    );
    let (_d, store, router) = setup(&mock).await;
    let Err(e) = router.stream(req(&store).await, CancellationToken::new()).await else {
        panic!()
    };
    assert_eq!(e.class(), ErrorClass::Overloaded);
    let (err, ..) = attempt_row(&store);
    assert_eq!(err.as_deref(), Some("overloaded"));
    // M2 never retries (that's M3): exactly one request.
    assert_eq!(mock.requests().len(), 1);
}
