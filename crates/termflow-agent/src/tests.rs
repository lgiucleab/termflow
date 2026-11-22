#![allow(clippy::unwrap_used)] // test code

use std::time::Duration;

use mock_provider::{MockProvider, Script};
use termflow_secrets::{MemoryStore, Redactor, Secrets};
use termflow_types::{Credential, ProviderKind, WireFormat};

use super::*;

const OK: &str = include_str!("../../../tests/fixtures/http/anthropic/text_thinking_tool.sse");
const ERR: &str = include_str!("../../../tests/fixtures/http/anthropic/error_after_tokens.sse");

struct H {
    _d: tempfile::TempDir,
    store: Store,
    chat: Chat,
    rx: broadcast::Receiver<AppEvent>,
    mock: MockProvider,
}

async fn h() -> H {
    let mock = MockProvider::start().await.unwrap();
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("t.db")).unwrap();
    let secrets = Secrets::new(Arc::new(MemoryStore::default()), Redactor::new());
    secrets.set("c1", "sk-ant-test-0000000000").unwrap();
    let cred = Credential {
        id: "c1".into(),
        kind: ProviderKind::Anthropic,
        label: "t".into(),
        base_url: mock.url(),
        wire: WireFormat::AnthropicMessages,
        has_secret: true,
        created_at: jiff::Timestamp::now(),
    };
    store
        .write(move |c| termflow_store::agent::insert_credential(c, &cred))
        .await
        .unwrap();
    let (bus, rx) = broadcast::channel(1024);
    let chat = Chat::new(store.clone(), Arc::new(Router::new(store.clone(), secrets)), bus);
    H {
        _d: d,
        store,
        chat,
        rx,
        mock,
    }
}

async fn wait<F: Fn(&AppEvent) -> bool>(rx: &mut broadcast::Receiver<AppEvent>, f: F) -> AppEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let e = rx.recv().await.unwrap();
            if f(&e) {
                return e;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn chat_streams_and_commits_assistant_turn() {
    let mut h = h().await;
    h.mock.push(Script::sse(OK));
    let s = h
        .chat
        .create_session(Some("c1".into()), Some("claude-test-model".into()))
        .await
        .unwrap();
    h.chat.send(s.id, "Read main.rs".into()).await.unwrap();
    let e = wait(
        &mut h.rx,
        |e| matches!(e, AppEvent::AgentTurnCommitted { turn } if turn.role == Role::Assistant),
    )
    .await;
    let AppEvent::AgentTurnCommitted { turn } = e else {
        unreachable!()
    };
    assert_eq!(turn.text(), "Let me look.");
    assert!(
        matches!(&turn.parts[0], Part::Reasoning { family, credential_scope, .. } if family == "anthropic" && credential_scope == "c1")
    );
    assert!(matches!(&turn.parts[2], Part::ToolCall { name, .. } if name == "read"));
    assert_eq!(turn.served_by.as_ref().unwrap().model, "claude-test-model");
    // Persisted: user + assistant, and the session got a title.
    assert_eq!(h.chat.transcript(s.id).unwrap().len(), 2);
    assert_eq!(h.chat.session(s.id).unwrap().title.as_deref(), Some("Read main.rs"));

    // Second message: the thinking block goes back verbatim to the same family/credential.
    h.mock.push(Script::sse(OK));
    h.chat.send(s.id, "and?".into()).await.unwrap();
    wait(
        &mut h.rx,
        |e| matches!(e, AppEvent::AgentTurnCommitted { turn } if turn.role == Role::Assistant),
    )
    .await;
    let sent = &h.mock.requests()[1].body;
    assert_eq!(sent["messages"][1]["content"][0]["signature"], "sig-test-abc");
}

#[tokio::test]
async fn midstream_error_discards_partial_and_reports() {
    let mut h = h().await;
    h.mock.push(Script::sse(ERR));
    let s = h
        .chat
        .create_session(Some("c1".into()), Some("m".into()))
        .await
        .unwrap();
    h.chat.send(s.id, "hi".into()).await.unwrap();
    let e = wait(&mut h.rx, |e| matches!(e, AppEvent::AgentError { .. })).await;
    assert!(matches!(e, AppEvent::AgentError { class, .. } if class == "overloaded"));
    // Only the user turn is committed; the partial assistant turn is discarded, not resent.
    assert_eq!(h.chat.transcript(s.id).unwrap().len(), 1);
    let discarded: i64 = h
        .store
        .reader()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM turns WHERE status='discarded'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(discarded, 1);
}

#[tokio::test]
async fn cancel_keeps_partial_text() {
    let mut h = h().await;
    h.mock.push(Script::sse(OK).stall_after(8));
    let s = h
        .chat
        .create_session(Some("c1".into()), Some("m".into()))
        .await
        .unwrap();
    h.chat.send(s.id, "hi".into()).await.unwrap();
    wait(&mut h.rx, |e| {
        matches!(e, AppEvent::AgentDelta { reasoning: false, .. })
    })
    .await;
    h.chat.cancel(s.id);
    let e = wait(
        &mut h.rx,
        |e| matches!(e, AppEvent::AgentTurnCommitted { turn } if turn.role == Role::Assistant),
    )
    .await;
    let AppEvent::AgentTurnCommitted { turn } = e else {
        unreachable!()
    };
    assert!(turn.text().starts_with("Let me"));
    assert!(turn.text().ends_with("[cancelled]"));
    assert!(turn.parts.iter().all(|p| matches!(p, Part::Text { .. })));
}

#[tokio::test]
async fn send_without_model_is_refused() {
    let h = h().await;
    let s = h.chat.create_session(None, None).await.unwrap();
    assert!(matches!(h.chat.send(s.id, "hi".into()).await, Err(AgentError::NoModel)));
}
