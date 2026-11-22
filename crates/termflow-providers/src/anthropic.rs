//! Anthropic Messages API (`anthropic-version: 2026-10-21`).

use std::collections::HashMap;

use serde_json::{Value, json};
use termflow_types::{ModelInfo, Part, Role, StopReason, StreamEvent, Turn, Usage, WireFormat};
use tokio_util::sync::CancellationToken;

use crate::sse::{self, Decoder, Framing, RawEvent};
use crate::{EventStream, ProviderAdapter, ProviderError, RenderContext, Target, join, text_of};

pub const API_VERSION: &str = "2026-10-21";

pub struct Anthropic {
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    fn req(&self, target: &Target, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self
            .http
            .request(method, join(&target.base_url, path))
            .header("anthropic-version", API_VERSION);
        if let Some(k) = &target.api_key {
            r = r.header("x-api-key", k);
        }
        r
    }
}

/// Append blocks to the last message if it has the same role (the API requires alternation).
fn push(messages: &mut Vec<Value>, role: &str, mut blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut()
        && last["role"] == role
        && let Some(arr) = last["content"].as_array_mut()
    {
        arr.append(&mut blocks);
        return;
    }
    messages.push(json!({ "role": role, "content": blocks }));
}

pub fn render(transcript: &[Turn], ctx: &RenderContext) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for t in transcript {
        let mut blocks = Vec::new();
        for p in &t.parts {
            match p {
                Part::Text { text } if !text.is_empty() => blocks.push(json!({"type":"text","text":text})),
                // Kept verbatim and in place; other families' reasoning is dropped (§7.1).
                Part::Reasoning { native, .. } if ctx.may_resend(p) => blocks.push(native.clone()),
                Part::Text { .. } | Part::Reasoning { .. } => {}
                Part::ToolCall { call_id, name, args } => {
                    blocks.push(json!({"type":"tool_use","id":call_id.to_string(),"name":name,"input":args}));
                }
                Part::ToolResult { call_id, content, is_error } => blocks.push(json!({
                    "type":"tool_result","tool_use_id":call_id.to_string(),"content":text_of(content),"is_error":is_error
                })),
            }
        }
        let role = match t.role {
            Role::Assistant => "assistant",
            Role::User | Role::Tool | Role::System => "user",
        };
        push(&mut messages, role, blocks);
    }
    // Rolling cache breakpoint on the newest user content.
    if let Some(last_user) = messages.iter_mut().rev().find(|m| m["role"] == "user")
        && let Some(block) = last_user["content"].as_array_mut().and_then(|a| a.last_mut())
    {
        block["cache_control"] = json!({"type":"ephemeral"});
    }
    let mut body = json!({
        "model": ctx.model,
        "max_tokens": ctx.max_tokens,
        "stream": true,
        "messages": messages,
    });
    if !ctx.system.is_empty() {
        body["system"] = json!([{"type":"text","text":ctx.system,"cache_control":{"type":"ephemeral"}}]);
    }
    if ctx.reasoning {
        body["thinking"] = json!({"type":"adaptive","display":"summarized"});
    }
    if !ctx.tools.is_empty() {
        let mut tools: Vec<Value> = ctx
            .tools
            .iter()
            .map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.parameters}))
            .collect();
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type":"ephemeral"});
        }
        body["tools"] = Value::Array(tools);
        // Forced tool_choice is rejected on current models; "auto" everywhere.
        body["tool_choice"] = json!({"type":"auto"});
    }
    body
}

#[derive(Default)]
struct AnthropicDecoder {
    blocks: HashMap<u64, Block>,
    stop: Option<StopReason>,
    done: bool,
}

enum Block {
    Text,
    Thinking { text: String, signature: String },
    Redacted(Value),
    ToolUse { id: String },
}

fn u(v: &Value, k: &str) -> u32 {
    v.get(k)
        .and_then(Value::as_u64)
        .and_then(|x| u32::try_from(x).ok())
        .unwrap_or(0)
}

impl Decoder for AnthropicDecoder {
    fn decode(&mut self, ev: &RawEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if ev.data.is_empty() {
            return Ok(vec![]);
        }
        let v = sse::json(&ev.data)?;
        let kind = v["type"].as_str().unwrap_or(ev.event.as_str());
        let idx = v["index"].as_u64().unwrap_or(0);
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                let m = &v["message"];
                if let Some(model) = m["model"].as_str() {
                    out.push(StreamEvent::ServedModel(model.into()));
                }
                let us = &m["usage"];
                out.push(StreamEvent::Usage(Usage {
                    input: u(us, "input_tokens"),
                    output: u(us, "output_tokens"),
                    cache_write: u(us, "cache_creation_input_tokens"),
                    cache_read: u(us, "cache_read_input_tokens"),
                    reasoning: 0,
                }));
            }
            "content_block_start" => {
                let b = &v["content_block"];
                let block = match b["type"].as_str() {
                    Some("thinking") => Block::Thinking {
                        text: b["thinking"].as_str().unwrap_or_default().into(),
                        signature: b["signature"].as_str().unwrap_or_default().into(),
                    },
                    Some("redacted_thinking") => Block::Redacted(b.clone()),
                    Some("tool_use") => {
                        let id = b["id"].as_str().unwrap_or_default().to_owned();
                        out.push(StreamEvent::ToolCallStart {
                            native_id: id.clone(),
                            name: b["name"].as_str().unwrap_or_default().into(),
                        });
                        Block::ToolUse { id }
                    }
                    _ => Block::Text,
                };
                self.blocks.insert(idx, block);
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match (d["type"].as_str(), self.blocks.get_mut(&idx)) {
                    (Some("text_delta"), _) => {
                        out.push(StreamEvent::TextDelta(d["text"].as_str().unwrap_or_default().into()));
                    }
                    (Some("thinking_delta"), Some(Block::Thinking { text, .. })) => {
                        let t = d["thinking"].as_str().unwrap_or_default();
                        text.push_str(t);
                        out.push(StreamEvent::ReasoningDelta(t.into()));
                    }
                    (Some("signature_delta"), Some(Block::Thinking { signature, .. })) => {
                        signature.push_str(d["signature"].as_str().unwrap_or_default());
                    }
                    (Some("input_json_delta"), Some(Block::ToolUse { id })) => {
                        out.push(StreamEvent::ToolCallArgsDelta {
                            native_id: id.clone(),
                            json_fragment: d["partial_json"].as_str().unwrap_or_default().into(),
                        });
                    }
                    _ => {}
                }
            }
            "content_block_stop" => match self.blocks.remove(&idx) {
                Some(Block::Thinking { text, signature }) => out.push(StreamEvent::ReasoningDone {
                    native: json!({"type":"thinking","thinking":text,"signature":signature}),
                    summary_text: (!text.is_empty()).then_some(text),
                }),
                Some(Block::Redacted(native)) => out.push(StreamEvent::ReasoningDone {
                    native,
                    summary_text: None,
                }),
                Some(Block::ToolUse { id }) => out.push(StreamEvent::ToolCallEnd { native_id: id }),
                Some(Block::Text) | None => {}
            },
            "message_delta" => {
                self.stop = Some(match v["delta"]["stop_reason"].as_str() {
                    Some("end_turn" | "stop_sequence") => StopReason::EndTurn,
                    Some("tool_use") => StopReason::ToolUse,
                    Some("max_tokens") => StopReason::MaxTokens,
                    Some("refusal") => StopReason::Refusal,
                    _ => StopReason::Other,
                });
                out.push(StreamEvent::Usage(Usage {
                    output: u(&v["usage"], "output_tokens"),
                    ..Usage::default()
                }));
            }
            "message_stop" => {
                self.done = true;
                out.push(StreamEvent::Done {
                    stop_reason: self.stop.unwrap_or(StopReason::EndTurn),
                });
            }
            "error" => return Err(sse::stream_error(WireFormat::AnthropicMessages, &v)),
            _ => {} // ping and future event types
        }
        Ok(out)
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for Anthropic {
    fn wire(&self) -> WireFormat {
        WireFormat::AnthropicMessages
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
            self.req(target, reqwest::Method::POST, "v1/messages").json(&body),
            self.wire(),
        )
        .await?;
        Ok(sse::drive(
            resp,
            Framing::Sse,
            AnthropicDecoder::default(),
            target.stall_timeout,
            cancel,
        ))
    }

    async fn list_models(&self, target: &Target) -> Result<Vec<ModelInfo>, ProviderError> {
        let resp = sse::send(
            self.req(target, reqwest::Method::GET, "v1/models?limit=1000"),
            self.wire(),
        )
        .await?;
        let v: Value = resp.json().await.map_err(|e| crate::transport(&e))?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: m["id"].as_str()?.into(),
                    display_name: m["display_name"].as_str().map(Into::into),
                    context_window: m["max_input_tokens"].as_u64().and_then(|x| u32::try_from(x).ok()),
                })
            })
            .collect())
    }
}
