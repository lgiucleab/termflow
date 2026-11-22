//! `OpenAI` Chat Completions and compatible servers (`OpenRouter`, Groq, `DeepSeek`, LM Studio, `vLLM`…).

use std::collections::BTreeMap;

use serde_json::{Value, json};
use termflow_types::{ModelInfo, Part, Role, StopReason, StreamEvent, Turn, Usage, WireFormat};
use tokio_util::sync::CancellationToken;

use crate::openai_responses::{bearer, list_openai_models};
use crate::sse::{self, Decoder, Framing, RawEvent};
use crate::{EventStream, ProviderAdapter, ProviderError, RenderContext, Target, text_of};

pub struct OpenAiChat {
    http: reqwest::Client,
}

impl OpenAiChat {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

pub fn render(transcript: &[Turn], ctx: &RenderContext, openrouter: bool) -> Value {
    let mut messages = Vec::new();
    if !ctx.system.is_empty() {
        messages.push(json!({"role":"system","content":ctx.system}));
    }
    for t in transcript {
        match t.role {
            Role::Assistant => {
                let text = t.text();
                let calls: Vec<Value> = t
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { call_id, name, args } => Some(json!({
                            "id":call_id.to_string(),"type":"function","function":{"name":name,"arguments":args.to_string()}
                        })),
                        _ => None,
                    })
                    .collect();
                let mut m =
                    json!({"role":"assistant","content": if text.is_empty() { Value::Null } else { json!(text) }});
                if !calls.is_empty() {
                    m["tool_calls"] = Value::Array(calls);
                }
                // OpenRouter: reasoning_details go back unchanged and in order.
                for p in &t.parts {
                    if let Part::Reasoning { native, .. } = p
                        && ctx.may_resend(p)
                        && let Some(d) = native.get("reasoning_details")
                    {
                        m["reasoning_details"] = d.clone();
                    }
                }
                messages.push(m);
            }
            Role::User | Role::Tool | Role::System => {
                for p in &t.parts {
                    match p {
                        Part::Text { text } if !text.is_empty() => messages.push(json!({"role":"user","content":text})),
                        Part::ToolResult { call_id, content, .. } => messages.push(json!({
                            "role":"tool","tool_call_id":call_id.to_string(),"content":text_of(content)
                        })),
                        _ => {}
                    }
                }
            }
        }
    }
    let mut body = json!({
        "model": ctx.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        // max_tokens, not max_completion_tokens: it's what most compatible servers accept.
        "max_tokens": ctx.max_tokens,
    });
    if openrouter && ctx.reasoning {
        body["reasoning"] = json!({"effort":"medium"});
    }
    if !ctx.tools.is_empty() {
        body["tools"] = Value::Array(
            ctx.tools
                .iter()
                .map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.parameters}}))
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }
    body
}

#[derive(Default)]
struct ChatDecoder {
    /// tool call index → id
    calls: BTreeMap<u64, String>,
    reasoning_details: BTreeMap<u64, Value>,
    finish: Option<StopReason>,
    saw_model: bool,
    done: bool,
}

/// Merge a streamed `reasoning_details` fragment into the item with the same index.
fn merge_detail(into: &mut Value, frag: &Value) {
    let (Some(dst), Some(src)) = (into.as_object_mut(), frag.as_object()) else {
        return;
    };
    for (k, v) in src {
        match (dst.get_mut(k), v) {
            (Some(Value::String(a)), Value::String(b)) if matches!(k.as_str(), "text" | "summary" | "data") => {
                a.push_str(b);
            }
            (Some(_), _) if k == "index" => {}
            _ => {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
}

fn u(v: &Value, path: &[&str]) -> u32 {
    path.iter()
        .try_fold(v, |acc, k| acc.get(*k))
        .and_then(Value::as_u64)
        .and_then(|x| u32::try_from(x).ok())
        .unwrap_or(0)
}

impl ChatDecoder {
    fn close(&mut self) -> Vec<StreamEvent> {
        let mut out: Vec<StreamEvent> = std::mem::take(&mut self.calls)
            .into_values()
            .map(|native_id| StreamEvent::ToolCallEnd { native_id })
            .collect();
        if !self.reasoning_details.is_empty() {
            let details: Vec<Value> = std::mem::take(&mut self.reasoning_details).into_values().collect();
            let summary: String = details
                .iter()
                .filter_map(|d| d["text"].as_str().or(d["summary"].as_str()))
                .collect();
            out.push(StreamEvent::ReasoningDone {
                native: json!({"reasoning_details": details}),
                summary_text: (!summary.is_empty()).then_some(summary),
            });
        }
        out
    }
}

impl Decoder for ChatDecoder {
    fn decode(&mut self, ev: &RawEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if ev.data.is_empty() {
            return Ok(vec![]);
        }
        if ev.data.trim() == "[DONE]" {
            let mut out = self.close();
            self.done = true;
            out.push(StreamEvent::Done {
                stop_reason: self.finish.unwrap_or(StopReason::EndTurn),
            });
            return Ok(out);
        }
        let v = sse::json(&ev.data)?;
        if v.get("error").is_some_and(|e| !e.is_null()) {
            return Err(sse::stream_error(WireFormat::OpenaiChat, &v));
        }
        let mut out = Vec::new();
        if !self.saw_model
            && let Some(m) = v["model"].as_str()
        {
            self.saw_model = true;
            out.push(StreamEvent::ServedModel(m.into()));
        }
        if let Some(choice) = v["choices"].get(0) {
            let d = &choice["delta"];
            if let Some(t) = d["content"].as_str().filter(|t| !t.is_empty()) {
                out.push(StreamEvent::TextDelta(t.into()));
            }
            if let Some(r) = d["reasoning"]
                .as_str()
                .or(d["reasoning_content"].as_str())
                .filter(|t| !t.is_empty())
            {
                out.push(StreamEvent::ReasoningDelta(r.into()));
            }
            for frag in d["reasoning_details"].as_array().into_iter().flatten() {
                let i = frag["index"].as_u64().unwrap_or(0);
                match self.reasoning_details.get_mut(&i) {
                    Some(existing) => merge_detail(existing, frag),
                    None => {
                        self.reasoning_details.insert(i, frag.clone());
                    }
                }
            }
            for tc in d["tool_calls"].as_array().into_iter().flatten() {
                let i = tc["index"].as_u64().unwrap_or(0);
                if let Some(id) = tc["id"].as_str().filter(|_| !self.calls.contains_key(&i)) {
                    self.calls.insert(i, id.into());
                    out.push(StreamEvent::ToolCallStart {
                        native_id: id.into(),
                        name: tc["function"]["name"].as_str().unwrap_or_default().into(),
                    });
                }
                if let (Some(id), Some(args)) = (self.calls.get(&i), tc["function"]["arguments"].as_str())
                    && !args.is_empty()
                {
                    out.push(StreamEvent::ToolCallArgsDelta {
                        native_id: id.clone(),
                        json_fragment: args.into(),
                    });
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                self.finish = Some(match f {
                    "tool_calls" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    "content_filter" => StopReason::ContentFilter,
                    "stop" => StopReason::EndTurn,
                    _ => StopReason::Other,
                });
            }
        }
        if v["usage"].is_object() {
            let us = &v["usage"];
            out.push(StreamEvent::Usage(Usage {
                input: u(us, &["prompt_tokens"]),
                output: u(us, &["completion_tokens"]),
                cache_read: u(us, &["prompt_tokens_details", "cached_tokens"]),
                cache_write: u(us, &["prompt_tokens_details", "cache_write_tokens"]),
                reasoning: u(us, &["completion_tokens_details", "reasoning_tokens"]),
            }));
        }
        Ok(out)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    /// Some compatible servers end the stream without `[DONE]` after a `finish_reason`.
    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        if self.done || self.finish.is_none() {
            return Ok(vec![]);
        }
        let mut out = self.close();
        self.done = true;
        out.push(StreamEvent::Done {
            stop_reason: self.finish.unwrap_or(StopReason::EndTurn),
        });
        Ok(out)
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenAiChat {
    fn wire(&self) -> WireFormat {
        WireFormat::OpenaiChat
    }

    fn render(&self, transcript: &[Turn], ctx: &RenderContext) -> Result<Value, ProviderError> {
        Ok(render(transcript, ctx, ctx.family == "openrouter"))
    }

    async fn stream(
        &self,
        target: &Target,
        body: Value,
        cancel: CancellationToken,
    ) -> Result<EventStream, ProviderError> {
        let mut req = bearer(&self.http, target, reqwest::Method::POST, "chat/completions").json(&body);
        if target.provider == "openrouter" {
            req = req.header("X-OpenRouter-Title", "TermFlow");
        }
        let resp = sse::send(req, self.wire()).await?;
        Ok(sse::drive(
            resp,
            Framing::Sse,
            ChatDecoder::default(),
            target.stall_timeout,
            cancel,
        ))
    }

    async fn list_models(&self, target: &Target) -> Result<Vec<ModelInfo>, ProviderError> {
        list_openai_models(&self.http, target, self.wire()).await
    }
}
