//! Agent tests against the mock provider, including the `BUILD_PROMPT` §12 M2 acceptance flow.
#![allow(clippy::unwrap_used)] // test code

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use mock_provider::{MockProvider, Script};
use serde_json::json;
use termflow_secrets::{MemoryStore, Redactor, Secrets};
use termflow_types::{Credential, ProviderKind, WireFormat};

use super::*;

const OK: &str = include_str!("../../../tests/fixtures/http/anthropic/text_thinking_tool.sse");
const ERR: &str = include_str!("../../../tests/fixtures/http/anthropic/error_after_tokens.sse");

// ---- scripted responses per wire format ----

fn sse(events: &[(&str, serde_json::Value)]) -> Script {
    let mut body = String::new();
    for (e, d) in events {
        let _ = write!(body, "event: {e}\ndata: {d}\n\n");
    }
    Script::sse(&body)
}

fn anthropic_tool(id: &str, name: &str, args: &serde_json::Value) -> Script {
    sse(&[
        (
            "message_start",
            json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":1}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":id,"name":name,"input":{}}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":args.to_string()}}),
        ),
        ("content_block_stop", json!({"type":"content_block_stop","index":0})),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ])
}

fn anthropic_text(text: &str) -> Script {
    sse(&[
        (
            "message_start",
            json!({"type":"message_start","message":{"model":"m","usage":{"input_tokens":10,"output_tokens":1}}}),
        ),
        (
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        (
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        ),
        ("content_block_stop", json!({"type":"content_block_stop","index":0})),
        (
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
        ),
        ("message_stop", json!({"type":"message_stop"})),
    ])
}

fn responses_tool(id: &str, name: &str, args: &serde_json::Value) -> Script {
    let item = json!({"id":format!("fc_{id}"),"type":"function_call","call_id":id,"name":name,"arguments":""});
    sse(&[
        (
            "response.created",
            json!({"type":"response.created","response":{"model":"gpt-x"}}),
        ),
        (
            "response.output_item.added",
            json!({"type":"response.output_item.added","output_index":0,"item":item}),
        ),
        (
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","item_id":format!("fc_{id}"),"output_index":0,"delta":args.to_string()}),
        ),
        (
            "response.output_item.done",
            json!({"type":"response.output_item.done","output_index":0,"item":{"id":format!("fc_{id}"),"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}}),
        ),
        (
            "response.completed",
            json!({"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}),
        ),
    ])
}

fn responses_text(text: &str) -> Script {
    sse(&[
        (
            "response.created",
            json!({"type":"response.created","response":{"model":"gpt-x"}}),
        ),
        (
            "response.output_text.delta",
            json!({"type":"response.output_text.delta","item_id":"m","output_index":0,"content_index":0,"delta":text}),
        ),
        (
            "response.completed",
            json!({"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}),
        ),
    ])
}

fn ollama_tool(_id: &str, name: &str, args: &serde_json::Value) -> Script {
    Script::ndjson(&format!(
        "{}\n{}\n",
        json!({"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":name,"arguments":args}}]},"done":false}),
        json!({"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":10,"eval_count":5})
    ))
}

fn ollama_text(text: &str) -> Script {
    Script::ndjson(&format!(
        "{}\n{}\n",
        json!({"message":{"role":"assistant","content":text},"done":false}),
        json!({"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":10,"eval_count":5})
    ))
}

// ---- harness ----

struct H {
    _d: tempfile::TempDir,
    store: Store,
    chat: Chat,
    rx: broadcast::Receiver<AppEvent>,
    mock: MockProvider,
    router: Router,
    project: PathBuf,
    workspace: String,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

async fn h(wire: WireFormat) -> H {
    let mock = MockProvider::start().await.unwrap();
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(d.path().join("t.db")).unwrap();
    let project = d.path().canonicalize().unwrap().join("sample");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample-rust-repo"),
        &project,
    );
    let secrets = Secrets::new(Arc::new(MemoryStore::default()), Redactor::new());
    secrets.set("c1", "sk-test-0000000000").unwrap();
    let (kind, base) = match wire {
        WireFormat::AnthropicMessages => (ProviderKind::Anthropic, mock.url()),
        WireFormat::OpenaiResponses => (ProviderKind::Openai, format!("{}/v1", mock.url())),
        WireFormat::OpenaiChat => (ProviderKind::Openrouter, format!("{}/v1", mock.url())),
        WireFormat::Ollama => (ProviderKind::Ollama, mock.url()),
    };
    let cred = Credential {
        id: "c1".into(),
        kind,
        label: "t".into(),
        base_url: base,
        wire,
        has_secret: wire != WireFormat::Ollama,
        created_at: jiff::Timestamp::now(),
    };
    let p = project.display().to_string();
    let ws = store
        .write(move |c| {
            termflow_store::agent::insert_credential(c, &cred)?;
            termflow_store::agent::open_workspace(c, &p)
        })
        .await
        .unwrap();
    let (bus, rx) = broadcast::channel(4096);
    let opts = ChatOptions {
        rules: termflow_perms::default_rules(),
        ..ChatOptions::default()
    };
    let router = Router::new(store.clone(), secrets);
    let chat = Chat::new(store.clone(), Arc::new(router.clone()), bus, opts);
    H {
        _d: d,
        store,
        chat,
        rx,
        mock,
        router,
        project,
        workspace: ws.id,
    }
}

impl H {
    async fn session(&self, model: &str) -> AgentSession {
        self.chat
            .create_session(Some(self.workspace.clone()), Some("c1".into()), Some(model.into()))
            .await
            .unwrap()
    }

    /// Wait for `AgentIdle`; answer every approval card with `answer`. Returns all events seen.
    async fn run_to_idle(&mut self, answer: Option<Approval>) -> Vec<AppEvent> {
        let mut seen = Vec::new();
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                let e = self.rx.recv().await.unwrap();
                if let (AppEvent::ToolApprovalNeeded { call_id, .. }, Some(a)) = (&e, &answer) {
                    assert!(self.chat.approve(*call_id, a.clone()));
                }
                let idle = matches!(e, AppEvent::AgentIdle { .. });
                seen.push(e);
                if idle {
                    return;
                }
            }
        })
        .await
        .unwrap();
        seen
    }
}

fn final_text(turns: &[Turn]) -> String {
    turns
        .iter()
        .rev()
        .find(|t| t.role == Role::Assistant)
        .map(Turn::text)
        .unwrap_or_default()
}

fn tool_results(turns: &[Turn]) -> Vec<(String, bool)> {
    turns
        .iter()
        .flat_map(|t| &t.parts)
        .filter_map(|p| match p {
            Part::ToolResult { content, is_error, .. } => {
                let ToolResultContent::Text { text } = &content[0];
                Some((text.clone(), *is_error))
            }
            _ => None,
        })
        .collect()
}

type ToolScript = fn(&str, &str, &serde_json::Value) -> Script;
type TextScript = fn(&str) -> Script;

/// `BUILD_PROMPT` §12 M2: reads files, proposes a change, runs `cargo test` after approval, reports.
async fn acceptance(wire: WireFormat, tool: ToolScript, text: TextScript) {
    let mut h = h(wire).await;
    h.mock.push(tool("call_1", "read", &json!({"path":"src/lib.rs"})));
    h.mock.push(tool(
        "call_2",
        "edit",
        &json!({"path":"src/lib.rs","old_string":"a - b","new_string":"a + b"}),
    ));
    h.mock.push(tool("call_3", "bash", &json!({"command":"cargo test"})));
    h.mock.push(text("Fixed `add`; all tests pass."));
    let s = h.session("test-model").await;
    h.chat
        .send(s.id, "The add test fails. Fix it and run the tests.".into())
        .await
        .unwrap();
    let events = h.run_to_idle(Some(Approval::AllowOnce)).await;

    // The edit asked for approval with a diff; read and `cargo test` didn't need to.
    let approvals: Vec<&AppEvent> = events
        .iter()
        .filter(|e| matches!(e, AppEvent::ToolApprovalNeeded { .. }))
        .collect();
    assert_eq!(approvals.len(), 1, "{approvals:?}");
    assert!(
        matches!(approvals[0], AppEvent::ToolApprovalNeeded { tool, preview: Some(p), .. } if tool == "edit" && p.contains("+    a + b")),
        "{approvals:?}"
    );

    assert!(
        std::fs::read_to_string(h.project.join("src/lib.rs"))
            .unwrap()
            .contains("a + b")
    );
    let turns = h.chat.transcript(s.id).unwrap();
    let results = tool_results(&turns);
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(results[0].0.contains("pub fn add"));
    assert!(results[1].0.starts_with("Edited src/lib.rs"));
    assert!(results[2].0.contains("test result: ok"), "{}", results[2].0);
    assert!(results.iter().all(|(_, err)| !err), "{results:?}");
    assert_eq!(final_text(&turns), "Fixed `add`; all tests pass.");
    // Each call ran exactly once.
    let r = h.store.reader().unwrap();
    let counts: Vec<i64> = r
        .prepare("SELECT exec_count FROM tool_calls")
        .unwrap()
        .query_map([], |x| x.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(counts, vec![1, 1, 1]);
    // 4 model requests; later ones carry the tool results back under the provider's own ids.
    let reqs = h.mock.requests();
    assert_eq!(reqs.len(), 4);
    if wire != WireFormat::Ollama {
        assert!(reqs[1].body.to_string().contains("call_1"), "{}", reqs[1].body);
    }
    assert!(reqs[0].body.to_string().contains("\"apply_patch\""), "tools offered");
}

#[tokio::test(flavor = "multi_thread")]
async fn acceptance_anthropic() {
    acceptance(WireFormat::AnthropicMessages, anthropic_tool, anthropic_text).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acceptance_openai_responses() {
    acceptance(WireFormat::OpenaiResponses, responses_tool, responses_text).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn acceptance_ollama() {
    acceptance(WireFormat::Ollama, ollama_tool, ollama_text).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_mode_rejects_edit_without_asking() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(anthropic_tool(
        "call_e",
        "edit",
        &json!({"path":"src/lib.rs","old_string":"a - b","new_string":"a + b"}),
    ));
    h.mock.push(anthropic_text("Plan: change `-` to `+`."));
    let s = h.session("m").await;
    h.chat.set_mode(s.id, true).await.unwrap();
    h.chat.send(s.id, "fix it".into()).await.unwrap();
    let events = h.run_to_idle(None).await;
    assert!(!events.iter().any(|e| matches!(e, AppEvent::ToolApprovalNeeded { .. })));
    let results = tool_results(&h.chat.transcript(s.id).unwrap());
    assert!(results[0].1 && results[0].0.contains("Plan mode"), "{results:?}");
    assert!(
        std::fs::read_to_string(h.project.join("src/lib.rs"))
            .unwrap()
            .contains("a - b")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn denied_call_returns_feedback_to_model() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(anthropic_tool(
        "call_w",
        "write",
        &json!({"path":"x.txt","content":"x"}),
    ));
    h.mock.push(anthropic_text("OK, not writing."));
    let s = h.session("m").await;
    h.chat.send(s.id, "write x".into()).await.unwrap();
    h.run_to_idle(Some(Approval::Deny {
        feedback: Some("use y.txt".into()),
    }))
    .await;
    let results = tool_results(&h.chat.transcript(s.id).unwrap());
    assert!(results[0].1 && results[0].0.contains("denied") && results[0].0.contains("use y.txt"));
    assert!(!h.project.join("x.txt").exists());
    assert!(h.mock.requests()[1].body.to_string().contains("use y.txt"));
}

/// A tool that started is never run again (`BUILD_PROMPT` §7.7.3), even after a restart.
#[tokio::test(flavor = "multi_thread")]
async fn interrupted_tool_is_repaired_not_rerun() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    let s = h.session("m").await;
    // Simulate a crash after the tool started: call committed, exec_count = 1, no result.
    let call = CallId::new();
    let turn = Turn::new(
        s.id,
        Role::Assistant,
        vec![Part::ToolCall {
            call_id: call,
            name: "bash".into(),
            args: json!({"command":"touch ran.txt"}),
        }],
        TurnStatus::Committed,
    );
    let t = turn.clone();
    let sid = s.id;
    h.store
        .write(move |c| {
            termflow_store::agent::append_turn(c, &t)?;
            termflow_store::agent::insert_tool_calls(
                c,
                sid,
                t.id,
                "c1",
                &[(call, "toolu_x".into(), "bash".into(), json!({}))],
            )?;
            termflow_store::agent::begin_tool_exec(c, call).map(|_| ())
        })
        .await
        .unwrap();
    h.mock.push(anthropic_text("Noted."));
    h.chat.send(s.id, "continue".into()).await.unwrap();
    h.run_to_idle(None).await;
    let turns = h.chat.transcript(s.id).unwrap();
    // Result appended right after the call (before the new user message), as an error; not re-run.
    assert_eq!(turns[1].role, Role::Tool);
    assert!(tool_results(&turns)[0].0.contains("not re-run"));
    assert!(!h.project.join("ran.txt").exists());
    assert_eq!(
        termflow_store::agent::exec_count(&h.store.reader().unwrap(), call).unwrap(),
        1
    );
    // The request is valid: tool_use followed by its tool_result, under the native id.
    let body = &h.mock.requests()[0].body;
    assert_eq!(body["messages"][0]["content"][0]["type"], "tool_use");
    assert_eq!(body["messages"][0]["content"][0]["id"], "toolu_x");
    assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
    assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "toolu_x");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_without_project_has_no_tools() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(Script::sse(OK));
    let s = h
        .chat
        .create_session(None, Some("c1".into()), Some("m".into()))
        .await
        .unwrap();
    h.chat.send(s.id, "hi".into()).await.unwrap();
    h.run_to_idle(None).await;
    let body = &h.mock.requests()[0].body;
    assert!(body.get("tools").is_none());
    assert!(
        body["system"][0]["text"]
            .as_str()
            .unwrap()
            .contains("No project folder")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn midstream_error_discards_partial_and_reports() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(Script::sse(ERR));
    let s = h.session("m").await;
    h.chat.send(s.id, "hi".into()).await.unwrap();
    let events = h.run_to_idle(None).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AppEvent::AgentError { class, .. } if class == "overloaded"))
    );
    assert_eq!(h.chat.transcript(s.id).unwrap().len(), 1);
}

/// `BUILD_PROMPT` §12 M3 (c) + (f): an SSE error after a 200 and some tokens hands the step to
/// the next model in the profile. The user types nothing, the step ends with exactly one
/// committed assistant turn, and the tool that already ran is not run again.
#[tokio::test(flavor = "multi_thread")]
async fn midstream_failure_hands_off_to_the_next_model_without_rerunning_tools() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    let backup = MockProvider::start().await.unwrap();
    let cred = Credential {
        id: "c2".into(),
        kind: ProviderKind::Openai,
        label: "backup".into(),
        base_url: format!("{}/v1", backup.url()),
        wire: WireFormat::OpenaiResponses,
        has_secret: false,
        created_at: jiff::Timestamp::now(),
    };
    h.store
        .write(move |c| termflow_store::agent::insert_credential(c, &cred))
        .await
        .unwrap();
    h.router.set_profiles(
        [(
            "main".to_owned(),
            termflow_config::Profile {
                chain: vec!["anthropic/m@t".into(), "openai/gpt-x@backup".into()],
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
    );
    h.mock
        .push(anthropic_tool("call_1", "read", &json!({"path":"src/lib.rs"})));
    h.mock.push(Script::sse(ERR)); // second step dies mid-stream
    backup.push(responses_text("The bug is `a - b`."));
    let s = h.session("m").await;
    h.chat.set_profile(s.id, Some("main".into())).await.unwrap();
    h.chat.send(s.id, "why does add fail?".into()).await.unwrap();
    let events = h.run_to_idle(None).await;

    assert!(
        !events.iter().any(|e| matches!(e, AppEvent::AgentError { .. })),
        "no error shown: {events:?}"
    );
    let turns = h.chat.transcript(s.id).unwrap();
    let roles: Vec<Role> = turns.iter().map(|t| t.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool, Role::Assistant]);
    assert!(turns.iter().all(|t| t.status == TurnStatus::Committed));
    assert_eq!(final_text(&turns), "The bug is `a - b`.");
    assert_eq!(
        turns[3].served_by.as_ref().unwrap().credential_id.as_deref(),
        Some("c2")
    );
    // The discarded partial turn stays in the database, out of the transcript.
    let discarded: i64 = h
        .store
        .reader()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM turns WHERE status = 'discarded'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(discarded, 1);
    // (f) the read ran once; the backup got its result instead of re-running it.
    let Part::ToolCall { call_id, .. } = turns[1].parts[0] else {
        panic!()
    };
    assert_eq!(
        termflow_store::agent::exec_count(&h.store.reader().unwrap(), call_id).unwrap(),
        1
    );
    let body = backup.requests()[0].body.to_string();
    assert!(body.contains("pub fn add"), "{body}");
    assert_eq!((h.mock.requests().len(), backup.requests().len()), (2, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_keeps_partial_text() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(Script::sse(OK).stall_after(8));
    let s = h.session("m").await;
    h.chat.send(s.id, "hi".into()).await.unwrap();
    loop {
        if matches!(
            h.rx.recv().await.unwrap(),
            AppEvent::AgentDelta { reasoning: false, .. }
        ) {
            break;
        }
    }
    h.chat.cancel(s.id);
    h.run_to_idle(None).await;
    let t = final_text(&h.chat.transcript(s.id).unwrap());
    assert!(t.starts_with("Let me") && t.ends_with("[cancelled]"), "{t}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mentions_attach_project_files() {
    let mut h = h(WireFormat::AnthropicMessages).await;
    h.mock.push(anthropic_text("Seen."));
    let s = h.session("m").await;
    h.chat.send(s.id, "explain @src/lib.rs".into()).await.unwrap();
    h.run_to_idle(None).await;
    let body = h.mock.requests()[0].body.clone();
    let user = body["messages"][0]["content"][0]["text"].as_str().unwrap().to_owned();
    assert!(user.contains("<attached_file path=\"src/lib.rs\">"), "{user}");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_without_model_is_refused() {
    let h = h(WireFormat::AnthropicMessages).await;
    let s = h
        .chat
        .create_session(Some(h.workspace.clone()), None, None)
        .await
        .unwrap();
    assert!(matches!(h.chat.send(s.id, "hi".into()).await, Err(AgentError::NoModel)));
}
