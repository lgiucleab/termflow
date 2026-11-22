//! Adapter tests: rendered-request snapshots (insta) and end-to-end streaming against the mock
//! provider using the fixtures in `tests/fixtures/http`.
#![allow(clippy::unwrap_used)] // test code

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use mock_provider::{MockProvider, Script};
use serde_json::json;
use termflow_types::{
    CallId, ErrorClass, Part, Role, SessionId, StopReason, StreamEvent, ToolResultContent, Turn, TurnStatus, Usage,
    WireFormat,
};
use tokio_util::sync::CancellationToken;

use super::*;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/http")
        .join(rel)
}

fn ctx(family: &str, scope: &str) -> RenderContext {
    RenderContext {
        model: "test-model".into(),
        system: "You are a test.".into(),
        max_tokens: 1024,
        reasoning: true,
        family: family.into(),
        credential_scope: scope.into(),
        tools: vec![ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        }],
    }
}

/// user → assistant(anthropic thinking + text + tool call) → tool result → user.
fn transcript() -> Vec<Turn> {
    let sid = SessionId::new();
    let call = CallId(ulid::Ulid::from_parts(0, 42));
    let mk = |role, parts| {
        let mut t = Turn::new(sid, role, parts, TurnStatus::Committed);
        t.created_at = jiff::Timestamp::UNIX_EPOCH;
        t
    };
    vec![
        mk(
            Role::User,
            vec![Part::Text {
                text: "Read main.rs".into(),
            }],
        ),
        mk(
            Role::Assistant,
            vec![
                Part::Reasoning {
                    family: "anthropic".into(),
                    model: "m".into(),
                    credential_scope: "cred-a".into(),
                    native: json!({"type":"thinking","thinking":"Check it","signature":"sig-1"}),
                    summary_text: Some("Check it".into()),
                },
                Part::Text {
                    text: "Reading.".into(),
                },
                Part::ToolCall {
                    call_id: call,
                    name: "read".into(),
                    args: json!({"path":"src/main.rs"}),
                },
            ],
        ),
        mk(
            Role::Tool,
            vec![Part::ToolResult {
                call_id: call,
                content: vec![ToolResultContent::Text {
                    text: "fn main() {}".into(),
                }],
                is_error: false,
            }],
        ),
        mk(
            Role::User,
            vec![Part::Text {
                text: "Explain it".into(),
            }],
        ),
    ]
}

#[test]
fn anthropic_render_same_scope_keeps_thinking() {
    insta::assert_json_snapshot!(anthropic::render(&transcript(), &ctx("anthropic", "cred-a")));
}

#[test]
fn anthropic_render_other_scope_drops_thinking() {
    let body = anthropic::render(&transcript(), &ctx("anthropic", "cred-b"));
    assert!(!body.to_string().contains("sig-1"));
    // Tool result and the next user text merge into one user message (strict alternation).
    let roles: Vec<&str> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
    assert!(body.get("temperature").is_none());
    assert_eq!(body["tool_choice"], json!({"type":"auto"}));
}

#[test]
fn openai_responses_render_drops_foreign_reasoning() {
    insta::assert_json_snapshot!(openai_responses::render(&transcript(), &ctx("openai", "cred-o")));
}

#[test]
fn openai_chat_render() {
    insta::assert_json_snapshot!(openai_chat::render(&transcript(), &ctx("openrouter", "cred-r"), true));
}

#[test]
fn ollama_render() {
    insta::assert_json_snapshot!(ollama::render(&transcript(), &ctx("ollama", "cred-l")));
}

fn target(mock: &MockProvider, wire: WireFormat, provider: &str) -> Target {
    let base = match wire {
        WireFormat::OpenaiResponses | WireFormat::OpenaiChat => format!("{}/v1", mock.url()),
        _ => mock.url(),
    };
    Target {
        provider: provider.into(),
        wire,
        base_url: base,
        model: "test-model".into(),
        credential_id: Some("cred".into()),
        api_key: Some("sk-test-key-123456".into()),
        stall_timeout: Duration::from_secs(5),
    }
}

async fn collect(
    wire: WireFormat,
    provider: &str,
    script: Script,
) -> (Vec<StreamEvent>, Option<ProviderError>, MockProvider) {
    let mock = MockProvider::start().await.unwrap();
    mock.push(script);
    let http = http_client();
    let a = adapter_for(wire, &http);
    let t = target(&mock, wire, provider);
    let body = a.render(&transcript(), &ctx(provider, "cred")).unwrap();
    let mut events = Vec::new();
    match a.stream(&t, body, CancellationToken::new()).await {
        Err(e) => return (events, Some(e), mock),
        Ok(mut s) => {
            while let Some(ev) = s.next().await {
                match ev {
                    Ok(e) => events.push(e),
                    Err(e) => return (events, Some(e), mock),
                }
            }
        }
    }
    (events, None, mock)
}

fn text(evs: &[StreamEvent]) -> String {
    evs.iter()
        .filter_map(|e| {
            if let StreamEvent::TextDelta(t) = e {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect()
}

fn args(evs: &[StreamEvent]) -> String {
    evs.iter()
        .filter_map(|e| {
            if let StreamEvent::ToolCallArgsDelta { json_fragment, .. } = e {
                Some(json_fragment.as_str())
            } else {
                None
            }
        })
        .collect()
}

fn usage(evs: &[StreamEvent]) -> Usage {
    let mut u = Usage::default();
    for e in evs {
        if let StreamEvent::Usage(x) = e {
            u.merge(*x);
        }
    }
    u
}

#[tokio::test]
async fn anthropic_stream_text_thinking_tool() {
    let (evs, err, mock) = collect(
        WireFormat::AnthropicMessages,
        "anthropic",
        Script::fixture(fixture("anthropic/text_thinking_tool.sse")).unwrap(),
    )
    .await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text(&evs), "Let me look.");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&args(&evs)).unwrap(),
        json!({"path":"src/main.rs"})
    );
    assert!(evs.contains(&StreamEvent::ReasoningDone {
        native: json!({"type":"thinking","thinking":"Check the file first.","signature":"sig-test-abc"}),
        summary_text: Some("Check the file first.".into()),
    }));
    assert!(evs.contains(&StreamEvent::ToolCallStart {
        native_id: "toolu_test1".into(),
        name: "read".into()
    }));
    assert!(evs.contains(&StreamEvent::ToolCallEnd {
        native_id: "toolu_test1".into()
    }));
    assert_eq!(
        usage(&evs),
        Usage {
            input: 120,
            output: 42,
            cache_write: 100,
            cache_read: 0,
            reasoning: 0
        }
    );
    assert_eq!(
        evs.last(),
        Some(&StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        })
    );
    // Request shape on the wire.
    let req = &mock.requests()[0];
    assert_eq!(req.path, "/v1/messages");
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "anthropic-version" && v == "2026-10-21")
    );
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "x-api-key" && v == "sk-test-key-123456")
    );
}

#[tokio::test]
async fn anthropic_error_event_after_tokens_is_classified() {
    let (evs, err, _) = collect(
        WireFormat::AnthropicMessages,
        "anthropic",
        Script::fixture(fixture("anthropic/error_after_tokens.sse")).unwrap(),
    )
    .await;
    assert_eq!(text(&evs), "Partial answ");
    assert_eq!(err.unwrap().class, ErrorClass::Overloaded);
}

#[tokio::test]
async fn anthropic_spend_limit_429() {
    let s = Script::fixture(fixture("anthropic/429_spend_limit.json"))
        .unwrap()
        .with_status(429);
    let (_, err, _) = collect(WireFormat::AnthropicMessages, "anthropic", s).await;
    let err = err.unwrap();
    assert_eq!(err.status, Some(429));
    assert!(matches!(err.class, ErrorClass::QuotaExhausted { reset_at: Some(_) }));
}

#[tokio::test]
async fn stalled_stream_is_overloaded() {
    let s = Script::fixture(fixture("anthropic/text_thinking_tool.sse"))
        .unwrap()
        .stall_after(3);
    let mock = MockProvider::start().await.unwrap();
    mock.push(s);
    let http = http_client();
    let a = adapter_for(WireFormat::AnthropicMessages, &http);
    let mut t = target(&mock, WireFormat::AnthropicMessages, "anthropic");
    t.stall_timeout = Duration::from_millis(300);
    let mut st = a.stream(&t, json!({}), CancellationToken::new()).await.unwrap();
    let mut last = None;
    while let Some(ev) = st.next().await {
        if let Err(e) = ev {
            last = Some(e);
        }
    }
    let e = last.unwrap();
    assert_eq!(e.class, ErrorClass::Overloaded);
    assert!(e.message.contains("stalled"));
}

#[tokio::test]
async fn cancellation_ends_stream() {
    let s = Script::fixture(fixture("anthropic/text_thinking_tool.sse"))
        .unwrap()
        .stall_after(2);
    let mock = MockProvider::start().await.unwrap();
    mock.push(s);
    let http = http_client();
    let a = adapter_for(WireFormat::AnthropicMessages, &http);
    let t = target(&mock, WireFormat::AnthropicMessages, "anthropic");
    let cancel = CancellationToken::new();
    let mut st = a.stream(&t, json!({}), cancel.clone()).await.unwrap();
    st.next().await;
    cancel.cancel();
    let mut saw_cancel = false;
    while let Some(ev) = st.next().await {
        if let Err(e) = ev {
            saw_cancel = e.class == ErrorClass::Cancelled;
        }
    }
    assert!(saw_cancel);
}

#[tokio::test]
async fn openai_responses_stream() {
    let (evs, err, mock) = collect(
        WireFormat::OpenaiResponses,
        "openai",
        Script::fixture(fixture("openai_responses/text_reasoning_tool.sse")).unwrap(),
    )
    .await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text(&evs), "Let me look.");
    assert_eq!(args(&evs), r#"{"path":"src/main.rs"}"#);
    assert!(evs.iter().any(
        |e| matches!(e, StreamEvent::ReasoningDone { native, .. } if native["encrypted_content"] == "enc-test-blob")
    ));
    assert!(evs.contains(&StreamEvent::ServedModel("gpt-test-model".into())));
    assert_eq!(
        usage(&evs),
        Usage {
            input: 200,
            output: 30,
            cache_read: 50,
            cache_write: 0,
            reasoning: 12
        }
    );
    assert_eq!(
        evs.last(),
        Some(&StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        })
    );
    let req = &mock.requests()[0];
    assert_eq!(req.path, "/v1/responses");
    assert_eq!(req.body["store"], json!(false));
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer sk-test-key-123456")
    );
}

#[tokio::test]
async fn openai_responses_failed_and_quota() {
    let (_, err, _) = collect(
        WireFormat::OpenaiResponses,
        "openai",
        Script::fixture(fixture("openai_responses/failed.sse")).unwrap(),
    )
    .await;
    assert_eq!(err.unwrap().class, ErrorClass::Overloaded);
    let s = Script::fixture(fixture("openai_responses/429_quota.json"))
        .unwrap()
        .with_status(429);
    let (_, err, _) = collect(WireFormat::OpenaiResponses, "openai", s).await;
    assert!(matches!(err.unwrap().class, ErrorClass::QuotaExhausted { .. }));
}

#[tokio::test]
async fn openai_chat_stream_with_openrouter_extras() {
    let (evs, err, mock) = collect(
        WireFormat::OpenaiChat,
        "openrouter",
        Script::fixture(fixture("openai_chat/text_tool.sse")).unwrap(),
    )
    .await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text(&evs), "Let me look.");
    assert_eq!(args(&evs), r#"{"path":"src/main.rs"}"#);
    assert!(evs.contains(&StreamEvent::ServedModel("served-test-model".into())));
    // reasoning_details fragments merged by index, signature kept.
    assert!(evs.iter().any(|e| matches!(e, StreamEvent::ReasoningDone { native, .. }
        if native["reasoning_details"][0]["text"] == "Think." && native["reasoning_details"][0]["signature"] == "sig-x")));
    assert_eq!(
        usage(&evs),
        Usage {
            input: 90,
            output: 20,
            cache_read: 10,
            cache_write: 0,
            reasoning: 5
        }
    );
    assert_eq!(
        evs.last(),
        Some(&StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        })
    );
    let req = &mock.requests()[0];
    assert_eq!(req.path, "/v1/chat/completions");
    assert_eq!(req.body["reasoning"], json!({"effort":"medium"}));
}

#[tokio::test]
async fn openai_chat_midstream_error() {
    let (evs, err, _) = collect(
        WireFormat::OpenaiChat,
        "openrouter",
        Script::fixture(fixture("openai_chat/midstream_error.sse")).unwrap(),
    )
    .await;
    assert_eq!(text(&evs), "Partial");
    assert_eq!(err.unwrap().class, ErrorClass::Overloaded);
}

#[tokio::test]
async fn ollama_stream() {
    let (evs, err, mock) = collect(
        WireFormat::Ollama,
        "ollama",
        Script::fixture(fixture("ollama/text_tool.ndjson")).unwrap(),
    )
    .await;
    assert!(err.is_none(), "{err:?}");
    assert_eq!(text(&evs), "Let me look.");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&args(&evs)).unwrap(),
        json!({"path":"src/main.rs"})
    );
    assert_eq!(
        usage(&evs),
        Usage {
            input: 50,
            output: 12,
            ..Usage::default()
        }
    );
    assert_eq!(
        evs.last(),
        Some(&StreamEvent::Done {
            stop_reason: StopReason::ToolUse
        })
    );
    assert_eq!(mock.requests()[0].path, "/api/chat");
}

#[tokio::test]
async fn list_models_each_wire() {
    let mock = MockProvider::start().await.unwrap();
    let http = http_client();
    mock.push(Script::fixture(fixture("anthropic/models.json")).unwrap());
    let m = adapter_for(WireFormat::AnthropicMessages, &http)
        .list_models(&target(&mock, WireFormat::AnthropicMessages, "anthropic"))
        .await
        .unwrap();
    assert_eq!(m[0].id, "claude-test-model");
    assert_eq!(m[0].context_window, Some(1_000_000));
    mock.push(Script::fixture(fixture("ollama/tags.json")).unwrap());
    let m = adapter_for(WireFormat::Ollama, &http)
        .list_models(&target(&mock, WireFormat::Ollama, "ollama"))
        .await
        .unwrap();
    assert_eq!(m[0].id, "local-test-model:latest");
    mock.push(Script::json(
        200,
        &json!({"data":[{"id":"b"},{"id":"a","context_length":8192}]}),
    ));
    let m = adapter_for(WireFormat::OpenaiChat, &http)
        .list_models(&target(&mock, WireFormat::OpenaiChat, "openrouter"))
        .await
        .unwrap();
    assert_eq!(m.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    mock.push(
        Script::json(
            401,
            &json!({"error":{"type":"authentication_error","message":"bad key"}}),
        )
        .with_status(401),
    );
    let e = adapter_for(WireFormat::AnthropicMessages, &http)
        .list_models(&target(&mock, WireFormat::AnthropicMessages, "anthropic"))
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrorClass::Auth);
}

#[test]
fn target_debug_hides_key() {
    let t = Target {
        provider: "x".into(),
        wire: WireFormat::OpenaiChat,
        base_url: "u".into(),
        model: "m".into(),
        credential_id: None,
        api_key: Some("sk-secret-should-not-print".into()),
        stall_timeout: Duration::from_secs(1),
    };
    assert!(!format!("{t:?}").contains("sk-secret"));
}
