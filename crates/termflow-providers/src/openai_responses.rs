//! `OpenAI` Responses API, stateless (`store: false`); reasoning items replayed verbatim.

use std::collections::HashMap;

use serde_json::{Value, json};
use termflow_types::{ModelInfo, Part, Role, StopReason, StreamEvent, Turn, Usage, WireFormat};
use tokio_util::sync::CancellationToken;

use crate::sse::{self, Decoder, Framing, RawEvent};
use crate::{EventStream, ProviderAdapter, ProviderError, RenderContext, Target, join, text_of};

pub struct OpenAiResponses {
    http: reqwest::Client,
}

impl OpenAiResponses {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }
}

pub(crate) fn bearer(
    http: &reqwest::Client,
    target: &Target,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let mut r = http.request(method, join(&target.base_url, path));
    if let Some(k) = &target.api_key {
        r = r.bearer_auth(k);
    }
    r
}

pub fn render(transcript: &[Turn], ctx: &RenderContext) -> Value {
    let mut input = Vec::new();
    for t in transcript {
        for p in &t.parts {
            match (t.role, p) {
                (Role::Assistant, Part::Text { text }) if !text.is_empty() => input.push(json!({
                    "type":"message","role":"assistant","content":[{"type":"output_text","text":text}]
                })),
                (_, Part::Text { text }) if !text.is_empty() => {
                    input.push(json!({"role":"user","content":[{"type":"input_text","text":text}]}));
                }
                (_, Part::Reasoning { native, .. }) if ctx.may_resend(p) => input.push(native.clone()),
                (_, Part::ToolCall { call_id, name, args }) => input.push(json!({
                    "type":"function_call","call_id":ctx.call_id(*call_id),"name":name,"arguments":args.to_string()
                })),
                (_, Part::ToolResult { call_id, content, .. }) => input.push(json!({
                    "type":"function_call_output","call_id":ctx.call_id(*call_id),"output":text_of(content)
                })),
                _ => {}
            }
        }
    }
    let mut body = json!({
        "model": ctx.model,
        "input": input,
        "stream": true,
        "store": false,
        "include": ["reasoning.encrypted_content"],
        "max_output_tokens": ctx.max_tokens,
    });
    if !ctx.system.is_empty() {
        body["instructions"] = json!(ctx.system);
    }
    if ctx.reasoning {
        body["reasoning"] = json!({"summary":"auto"});
    }
    if !ctx.tools.is_empty() {
        body["tools"] = Value::Array(
            ctx.tools
                .iter()
                .map(|t| json!({"type":"function","name":t.name,"description":t.description,"parameters":t.parameters}))
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }
    body
}

#[derive(Default)]
struct ResponsesDecoder {
    /// output item id → `call_id` (argument deltas reference the item id).
    calls: HashMap<String, String>,
    saw_tool_call: bool,
    done: bool,
}

fn u(v: &Value, path: &[&str]) -> u32 {
    path.iter()
        .try_fold(v, |acc, k| acc.get(*k))
        .and_then(Value::as_u64)
        .and_then(|x| u32::try_from(x).ok())
        .unwrap_or(0)
}

impl Decoder for ResponsesDecoder {
    fn decode(&mut self, ev: &RawEvent) -> Result<Vec<StreamEvent>, ProviderError> {
        if ev.data.is_empty() || ev.data == "[DONE]" {
            return Ok(vec![]);
        }
        let v = sse::json(&ev.data)?;
        let mut out = Vec::new();
        match v["type"].as_str().unwrap_or(ev.event.as_str()) {
            "response.created" => {
                if let Some(m) = v["response"]["model"].as_str() {
                    out.push(StreamEvent::ServedModel(m.into()));
                }
            }
            "response.output_item.added" => {
                let item = &v["item"];
                if item["type"] == "function_call" {
                    let call_id = item["call_id"].as_str().unwrap_or_default().to_owned();
                    self.calls
                        .insert(item["id"].as_str().unwrap_or_default().into(), call_id.clone());
                    self.saw_tool_call = true;
                    out.push(StreamEvent::ToolCallStart {
                        native_id: call_id,
                        name: item["name"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.output_text.delta" => {
                out.push(StreamEvent::TextDelta(v["delta"].as_str().unwrap_or_default().into()));
            }
            "response.reasoning_summary_text.delta" => {
                out.push(StreamEvent::ReasoningDelta(
                    v["delta"].as_str().unwrap_or_default().into(),
                ));
            }
            "response.function_call_arguments.delta" => {
                if let Some(call) = v["item_id"].as_str().and_then(|i| self.calls.get(i)) {
                    out.push(StreamEvent::ToolCallArgsDelta {
                        native_id: call.clone(),
                        json_fragment: v["delta"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.output_item.done" => {
                let item = &v["item"];
                match item["type"].as_str() {
                    Some("function_call") => {
                        out.push(StreamEvent::ToolCallEnd {
                            native_id: item["call_id"].as_str().unwrap_or_default().into(),
                        });
                    }
                    // Take reasoning from `.done`: only it has the final encrypted_content.
                    Some("reasoning") => {
                        let summary: String = item["summary"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|s| s["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        out.push(StreamEvent::ReasoningDone {
                            native: item.clone(),
                            summary_text: (!summary.is_empty()).then_some(summary),
                        });
                    }
                    _ => {}
                }
            }
            "response.completed" => {
                let us = &v["response"]["usage"];
                out.push(StreamEvent::Usage(Usage {
                    input: u(us, &["input_tokens"]),
                    output: u(us, &["output_tokens"]),
                    cache_read: u(us, &["input_tokens_details", "cached_tokens"]),
                    cache_write: u(us, &["input_tokens_details", "cache_write_tokens"]),
                    reasoning: u(us, &["output_tokens_details", "reasoning_tokens"]),
                }));
                self.done = true;
                let stop = if self.saw_tool_call {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                };
                out.push(StreamEvent::Done { stop_reason: stop });
            }
            "response.incomplete" => {
                self.done = true;
                let stop = match v["response"]["incomplete_details"]["reason"].as_str() {
                    Some("max_output_tokens") => StopReason::MaxTokens,
                    Some("content_filter") => StopReason::ContentFilter,
                    _ => StopReason::Other,
                };
                out.push(StreamEvent::Done { stop_reason: stop });
            }
            "response.failed" => {
                let err = json!({"error": v["response"]["error"].clone()});
                return Err(sse::stream_error(WireFormat::OpenaiResponses, &err));
            }
            "error" => return Err(sse::stream_error(WireFormat::OpenaiResponses, &v)),
            _ => {}
        }
        Ok(out)
    }

    fn is_done(&self) -> bool {
        self.done
    }
}

pub(crate) async fn list_openai_models(
    http: &reqwest::Client,
    target: &Target,
    wire: WireFormat,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let resp = sse::send(bearer(http, target, reqwest::Method::GET, "models"), wire).await?;
    let v: Value = resp.json().await.map_err(|e| crate::transport(&e))?;
    let mut models: Vec<ModelInfo> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            Some(ModelInfo {
                id: m["id"].as_str()?.into(),
                display_name: m["name"].as_str().map(Into::into),
                // OpenRouter adds context_length.
                context_window: m["context_length"].as_u64().and_then(|x| u32::try_from(x).ok()),
            })
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

#[async_trait::async_trait]
impl ProviderAdapter for OpenAiResponses {
    fn wire(&self) -> WireFormat {
        WireFormat::OpenaiResponses
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
        let req = bearer(&self.http, target, reqwest::Method::POST, "responses").json(&body);
        let resp = sse::send(req, self.wire()).await?;
        Ok(sse::drive(
            resp,
            Framing::Sse,
            ResponsesDecoder::default(),
            target.stall_timeout,
            cancel,
        ))
    }

    async fn list_models(&self, target: &Target) -> Result<Vec<ModelInfo>, ProviderError> {
        list_openai_models(&self.http, target, self.wire()).await
    }
}
