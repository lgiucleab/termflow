//! Shared transport: send, turn non-2xx into classified errors, and drive a decoder over an
//! SSE or NDJSON body with stall detection and cancellation.

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::HeaderMap;
use termflow_types::{ErrorClass, RateLimitSnapshot, StreamEvent, WireFormat};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::classify::{classify_http, error_message};
use crate::{EventStream, ProviderError, transport};

pub(crate) struct RawEvent {
    pub event: String,
    pub data: String,
}

pub(crate) trait Decoder: Send + 'static {
    fn decode(&mut self, ev: &RawEvent) -> Result<Vec<StreamEvent>, ProviderError>;
    /// The stream reached its terminal event.
    fn is_done(&self) -> bool;
    /// Called at EOF; providers without an explicit end marker finish here.
    fn finish(&mut self) -> Result<Vec<StreamEvent>, ProviderError> {
        Ok(vec![])
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Framing {
    Sse,
    Ndjson,
}

/// Send and return the response, or a classified error for non-2xx statuses.
pub(crate) async fn send(req: reqwest::RequestBuilder, wire: WireFormat) -> Result<reqwest::Response, ProviderError> {
    let resp = req.send().await.map_err(|e| transport(&e))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let headers = resp.headers().clone();
    let text = resp.text().await.unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    let class = classify_http(wire, Some(status.as_u16()), &headers, &body);
    let message = error_message(
        &body,
        &format!("HTTP {status}: {}", text.chars().take(300).collect::<String>()),
    );
    tracing::info!(status = status.as_u16(), class = class.name(), %message, "provider error");
    Err(ProviderError::new(class, Some(status.as_u16()), message))
}

/// Parse a rate-limit header that may hold a number of seconds, `6m0s`, or an RFC 3339 time.
fn parse_u64(h: &HeaderMap, k: &str) -> Option<u64> {
    h.get(k)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
}

pub(crate) fn rate_limits(h: &HeaderMap) -> Option<RateLimitSnapshot> {
    let snap = RateLimitSnapshot {
        requests_remaining: parse_u64(h, "anthropic-ratelimit-requests-remaining")
            .or_else(|| parse_u64(h, "x-ratelimit-remaining-requests")),
        tokens_remaining: parse_u64(h, "anthropic-ratelimit-tokens-remaining")
            .or_else(|| parse_u64(h, "x-ratelimit-remaining-tokens")),
        input_tokens_remaining: parse_u64(h, "anthropic-ratelimit-input-tokens-remaining"),
        output_tokens_remaining: parse_u64(h, "anthropic-ratelimit-output-tokens-remaining"),
        reset_at: h
            .get("anthropic-ratelimit-tokens-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok()),
        retry_after_secs: crate::retry_after_secs(h),
    };
    (snap != RateLimitSnapshot::default()).then_some(snap)
}

/// Drive `decoder` over the response body on a background task.
pub(crate) fn drive<D: Decoder>(
    resp: reqwest::Response,
    framing: Framing,
    mut decoder: D,
    stall: Duration,
    cancel: CancellationToken,
) -> EventStream {
    let (tx, rx) = mpsc::channel::<Result<StreamEvent, ProviderError>>(256);
    if let Some(rl) = rate_limits(resp.headers()) {
        let _ = tx.try_send(Ok(StreamEvent::RateLimit(rl)));
    }
    tokio::spawn(async move {
        let mut raw: futures::stream::BoxStream<'static, Result<RawEvent, ProviderError>> = match framing {
            Framing::Sse => resp
                .bytes_stream()
                .eventsource()
                .map(|r| {
                    r.map(|e| RawEvent {
                        event: e.event,
                        data: e.data,
                    })
                    .map_err(|e| ProviderError::new(ErrorClass::Overloaded, None, format!("stream read error: {e}")))
                })
                .boxed(),
            Framing::Ndjson => ndjson(resp).boxed(),
        };
        let fail = |e: ProviderError| async { tx.send(Err(e)).await.is_ok() };
        loop {
            let next = tokio::select! {
                () = cancel.cancelled() => {
                    let _ = fail(ProviderError::new(ErrorClass::Cancelled, None, "cancelled")).await;
                    return;
                }
                n = tokio::time::timeout(stall, raw.next()) => n,
            };
            match next {
                Err(_) => {
                    let msg = format!("stream stalled: no data for {}s", stall.as_secs());
                    let _ = fail(ProviderError::new(ErrorClass::Overloaded, None, msg)).await;
                    return;
                }
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    let _ = fail(e).await;
                    return;
                }
                Ok(Some(Ok(ev))) => match decoder.decode(&ev) {
                    Ok(events) => {
                        for e in events {
                            if tx.send(Ok(e)).await.is_err() {
                                return; // consumer went away
                            }
                        }
                        if decoder.is_done() {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = fail(e).await;
                        return;
                    }
                },
            }
        }
        match decoder.finish() {
            Ok(events) => {
                for e in events {
                    if tx.send(Ok(e)).await.is_err() {
                        return;
                    }
                }
                if !decoder.is_done() {
                    let _ = fail(ProviderError::new(
                        ErrorClass::Overloaded,
                        None,
                        "stream ended before completion",
                    ))
                    .await;
                }
            }
            Err(e) => {
                let _ = fail(e).await;
            }
        }
    });
    tokio_stream::wrappers::ReceiverStream::new(rx).boxed()
}

fn ndjson(resp: reqwest::Response) -> impl futures::Stream<Item = Result<RawEvent, ProviderError>> {
    let bytes = resp.bytes_stream();
    futures::stream::unfold((bytes, Vec::<u8>::new()), |(mut bytes, mut buf)| async move {
        loop {
            if let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let data = String::from_utf8_lossy(&line).trim().to_owned();
                if data.is_empty() {
                    continue;
                }
                return Some((
                    Ok(RawEvent {
                        event: String::new(),
                        data,
                    }),
                    (bytes, buf),
                ));
            }
            match bytes.next().await {
                Some(Ok(chunk)) => buf.extend_from_slice(&chunk),
                Some(Err(e)) => return Some((Err(transport(&e)), (bytes, buf))),
                None => {
                    let data = String::from_utf8_lossy(&std::mem::take(&mut buf)).trim().to_owned();
                    return (!data.is_empty()).then(|| {
                        (
                            Ok(RawEvent {
                                event: String::new(),
                                data,
                            }),
                            (bytes, buf),
                        )
                    });
                }
            }
        }
    })
}

/// Parse JSON event data, mapping garbage to a classified error rather than panicking.
pub(crate) fn json(data: &str) -> Result<serde_json::Value, ProviderError> {
    serde_json::from_str(data)
        .map_err(|e| ProviderError::new(ErrorClass::Unknown, None, format!("bad event JSON: {e}")))
}

/// Classify an error object that arrived inside a 200 stream.
pub(crate) fn stream_error(wire: WireFormat, body: &serde_json::Value) -> ProviderError {
    let class = classify_http(wire, None, &HeaderMap::new(), body);
    ProviderError::new(class, None, error_message(body, "error in stream"))
}
