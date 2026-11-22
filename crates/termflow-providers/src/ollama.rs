//! Ollama native API (`/api/chat`, NDJSON). Tool calls arrive whole and without ids.

use serde_json::{Value, json};
use termflow_types::{ModelInfo, Part, Role, StopReason, StreamEvent, Turn, Usage, WireFormat};
use tokio_util::sync::CancellationToken;

use crate::openai_responses::bearer;
use crate::sse::{self, Decoder, Framing, RawEvent};
use crate::{EventStream, ProviderAdapter, ProviderError, RenderContext, Target, text_of};

pub struct Ollama {
    http: reqwest::Client,
}

impl Ollama {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

pub fn render(transcript: &[Turn], ctx: &RenderContext) -> Value {
    let mut messages = Vec::new();
    if !ctx.system.is_empty() {
        messages.push(json!({"role":"system","content":ctx.system}));
    }
    // Ollama tool results name the tool, not the call id.
    let mut names = std::collections::HashMap::new();
    for t in transcript {
        match t.role {
            Role::Assistant => {
                let calls: Vec<Value> = t
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { call_id, name, args } => {
                            names.insert(*call_id, name.clone());
                            Some(json!({"function":{"name":name,"arguments":args}}))
                        }
                        _ => None,
                    })
                    .collect();
                let mut m = json!({"role":"assistant","content":t.text()});
                if !calls.is_empty() {
                    m["tool_calls"] = Value::Array(calls);
                }
                messages.push(m);
            }
            Role::User | Role::Tool | Role::System => {
                for p in &t.parts {
                    match p {
                        Part::Text { text } if !text.is_empty() => messages.push(json!({"role":"user","content":text})),
                        Part::ToolResult { call_id, content, .. } => messages.push(json!({
                            "role":"tool","content":text_of(content),"tool_name":names.get(call_id).cloned().unwrap_or_default()
                        })),
                        _ => {}
                    }
                }
            }
        }
    }
    let mut body = json!({"model": ctx.model, "messages": messages, "stream": true});
    if ctx.reasoning {
        body["think"] = json!(true);
    }
    if !ctx.tools.is_empty() {
        body["tools"] = Value::Array(
            ctx.tools
                .iter()
                .map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}}))
                .collect(),
        );
    }
    body
}

#[derive(Default)]
struct OllamaDecoder {
    saw_tool_call: bool,
    done: bool,
}

fn u(v: &Value, k: &str) -> u32 {
    v[k].as_u64().and_then(|x| u32::try_from(x).ok()).unwrap_or(0)
}

impl Decoder for OllamaDecoder {
    fn decode(&mut self, ev: &RawEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        let v = sse::json(&ev.data)?;
        if let Some(e) = v["error"].as_str() {
            return Err(sse::stream_error(WireFormat::Ollama, &json!({"error":{"message":e}})));
        }
        let mut out = Vec::new();
        let m = &v["message"];
        if let Some(t) = m["thinking"].as_str().filter(|t| !t.is_empty()) {
            out.push(StreamEvent::ReasoningDelta(t.into()));
        }
        if let Some(t) = m["content"].as_str().filter(|t| !t.is_empty()) {
            out.push(StreamEvent::TextDelta(t.into()));
        }
        for tc in m["tool_calls"].as_array().into_iter().flatten() {
            let id = format!("call_{}", ulid::Ulid::new());
            self.saw_tool_call = true;
            out.push(StreamEvent::ToolCallStart {
                native_id: id.clone(),
                name: tc["function"]["name"].as_str().unwrap_or_default().into(),
            });
            out.push(StreamEvent::ToolCallArgsDelta {
                native_id: id.clone(),
                json_fragment: tc["function"]["arguments"].to_string(),
            });
            out.push(StreamEvent::ToolCallEnd { native_id: id });
        }
        if v["done"].as_bool() == Some(true) {
            self.done = true;
            out.push(StreamEvent::Usage(Usage {
                input: u(&v, "prompt_eval_count"),
                output: u(&v, "eval_count"),
                ..Usage::default()
            }));
            let stop = if self.saw_tool_call {
                StopReason::ToolUse
            } else if v["done_reason"] == "length" {
                StopReason::MaxTokens
            } else {
                StopReason::EndTurn
            };
            out.push(StreamEvent::Done { stop_reason: stop });
        }
        Ok(out)
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Ollama {
    fn wire(&self) -> WireFormat {
        WireFormat::Ollama
    }

    fn render(&self, transcript: &[Turn], ctx: &RenderContext) -> Result<Value, ProviderError> {
        Ok(render(transcript, ctx))
    }

    async fn stream(
        &self,
        target: &Target,
        body: Value,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let resp = sse::send(
            bearer(&self.http, target, reqwest::Method::POST, "api/chat").json(&body),
            self.wire(),
        )
        .await?;
        Ok(sse::drive(
            resp,
            Framing::Ndjson,
            OllamaDecoder::default(),
            target.stall_timeout,
            cancel,
        ))
    }

    async fn list_models(&self, target: &Target) -> Result<Vec<ModelInfo>, ProviderError> {
        let resp = sse::send(
            bearer(&self.http, target, reqwest::Method::GET, "api/tags"),
            self.wire(),
        )
        .await?;
        let v: Value = resp.json().await.map_err(|e| crate::transport(&e))?;
        Ok(v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id: String = m["name"].as_str()?.into();
                let size = m["details"]["parameter_size"].as_str().map(|s| format!("{id} ({s})"));
                Some(ModelInfo {
                    id,
                    display_name: size,
                    context_window: None,
                })
            })
            .collect())
    }
}
