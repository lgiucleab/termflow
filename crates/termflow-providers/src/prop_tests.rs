#![allow(clippy::unwrap_used)] // test code

//! `BUILD_PROMPT` §12 M3: randomly generated tool-call histories always render to balanced
//! `tool_call`/`tool_result` pairs for every wire format, whatever model served earlier turns
//! (any switch sequence) and whoever renders now.

use std::collections::HashMap;

use proptest::prelude::*;
use termflow_types::{CallId, Part, Role, SessionId, ToolResultContent, Turn, TurnStatus, WireFormat};

use crate::{RenderContext, adapter_for, http_client};

const FAMILIES: [&str; 4] = ["anthropic", "openai", "openrouter", "ollama"];
const WIRES: [WireFormat; 4] = [
    WireFormat::AnthropicMessages,
    WireFormat::OpenaiResponses,
    WireFormat::OpenaiChat,
    WireFormat::Ollama,
];

/// One assistant step: reasoning family (if any), text?, number of tool calls, then a user line?
type Step = (Option<usize>, bool, usize, bool);

fn history(steps: &[Step], native_every: usize) -> (Vec<Turn>, HashMap<CallId, String>) {
    let s = SessionId::new();
    let mut natives = HashMap::new();
    let mut turns = vec![Turn::new(
        s,
        Role::User,
        vec![Part::Text { text: "start".into() }],
        TurnStatus::Committed,
    )];
    let mut n = 0usize;
    for (reasoning, text, calls, user) in steps {
        let mut parts = Vec::new();
        if let Some(f) = reasoning {
            parts.push(Part::Reasoning {
                family: FAMILIES[*f].into(),
                model: "m".into(),
                credential_scope: format!("cred-{f}"),
                native: serde_json::json!({"type":"thinking","thinking":"t","signature":"s"}),
                summary_text: Some("thought".into()),
            });
        }
        if *text {
            parts.push(Part::Text { text: "text".into() });
        }
        let ids: Vec<CallId> = (0..*calls).map(|_| CallId::new()).collect();
        for id in &ids {
            n += 1;
            if native_every > 0 && n.is_multiple_of(native_every) {
                natives.insert(*id, format!("native_{n}"));
            }
            parts.push(Part::ToolCall {
                call_id: *id,
                name: "read".into(),
                args: serde_json::json!({"path": "a"}),
            });
        }
        if parts.is_empty() {
            parts.push(Part::Text { text: "…".into() });
        }
        turns.push(Turn::new(s, Role::Assistant, parts, TurnStatus::Committed));
        if !ids.is_empty() {
            let results = ids
                .iter()
                .enumerate()
                .map(|(i, id)| Part::ToolResult {
                    call_id: *id,
                    content: vec![ToolResultContent::Text { text: "out".into() }],
                    is_error: i % 2 == 1,
                })
                .collect();
            turns.push(Turn::new(s, Role::Tool, results, TurnStatus::Committed));
        }
        if *user {
            turns.push(Turn::new(
                s,
                Role::User,
                vec![Part::Text { text: "more".into() }],
                TurnStatus::Committed,
            ));
        }
    }
    (turns, natives)
}

fn arr(v: &serde_json::Value, k: &str) -> Vec<serde_json::Value> {
    v.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default()
}
fn st(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned()
}

/// (call ids, result ids) in rendered order. Ollama has no ids: use positions.
fn pairs(wire: WireFormat, body: &serde_json::Value) -> (Vec<String>, Vec<String>) {
    let (mut calls, mut results) = (Vec::new(), Vec::new());
    match wire {
        WireFormat::AnthropicMessages => {
            for m in arr(body, "messages") {
                for c in arr(&m, "content") {
                    match st(&c, "type").as_str() {
                        "tool_use" => calls.push(st(&c, "id")),
                        "tool_result" => results.push(st(&c, "tool_use_id")),
                        _ => {}
                    }
                }
            }
        }
        WireFormat::OpenaiResponses => {
            for i in arr(body, "input") {
                match st(&i, "type").as_str() {
                    "function_call" => calls.push(st(&i, "call_id")),
                    "function_call_output" => results.push(st(&i, "call_id")),
                    _ => {}
                }
            }
        }
        WireFormat::OpenaiChat => {
            for m in arr(body, "messages") {
                calls.extend(arr(&m, "tool_calls").iter().map(|c| st(c, "id")));
                if st(&m, "role") == "tool" {
                    results.push(st(&m, "tool_call_id"));
                }
            }
        }
        WireFormat::Ollama => {
            for m in arr(body, "messages") {
                for _ in arr(&m, "tool_calls") {
                    calls.push(calls.len().to_string());
                }
                if st(&m, "role") == "tool" {
                    results.push(results.len().to_string());
                }
            }
        }
    }
    (calls, results)
}

fn step() -> impl Strategy<Value = Step> {
    (proptest::option::of(0usize..4), any::<bool>(), 0usize..4, any::<bool>())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]
    #[test]
    fn every_wire_renders_balanced_tool_pairs(
        steps in proptest::collection::vec(step(), 0..12),
        renderer in 0usize..4,
        native_every in 0usize..4,
    ) {
        let (turns, natives) = history(&steps, native_every);
        let expected: usize = steps.iter().map(|s| s.2).sum();
        let http = http_client();
        for wire in WIRES {
            let ctx = RenderContext {
                model: "m".into(),
                system: "s".into(),
                max_tokens: 100,
                reasoning: true,
                family: FAMILIES[renderer].into(),
                credential_scope: format!("cred-{renderer}"),
                tools: vec![],
                native_ids: natives.clone(),
            };
            let body = adapter_for(wire, &http).render(&turns, &ctx).unwrap();
            let (calls, results) = pairs(wire, &body);
            prop_assert_eq!(calls.len(), expected, "{:?}: {}", wire, body);
            prop_assert_eq!(&calls, &results, "{:?}: every call answered, in order", wire);
            let unique: std::collections::HashSet<_> = calls.iter().collect();
            prop_assert_eq!(unique.len(), calls.len(), "{:?}: ids unique", wire);
            // Native reasoning never leaks to another family/scope.
            let foreign = steps.iter().any(|s| s.0.is_some_and(|f| f != renderer));
            if foreign && wire == WireFormat::AnthropicMessages {
                let thinking = arr(&body, "messages").iter().flat_map(|m| arr(m, "content")).filter(|c| st(c, "type") == "thinking").count();
                let own = steps.iter().filter(|s| s.0 == Some(renderer)).count();
                prop_assert!(thinking <= own, "foreign thinking re-sent: {}", body);
            }
        }
    }
}
