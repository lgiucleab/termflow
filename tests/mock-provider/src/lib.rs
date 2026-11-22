//! A fake LLM provider for tests. Push scripted responses; every request (any path) pops the
//! next one. Responses can stream chunk by chunk with delays, or stall, to simulate real
//! failure modes. Requests are recorded so tests can assert on the rendered wire format.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::Response;
use futures::StreamExt;

#[derive(Debug, Clone)]
pub enum Chunk {
    Data(String),
    Delay(Duration),
    /// Hold the connection open without sending anything more (stream stall).
    Stall,
}

#[derive(Debug, Clone)]
pub struct Script {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<Chunk>,
}

impl Script {
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            chunks: vec![Chunk::Data(body.to_string())],
        }
    }

    /// An SSE body: events separated by blank lines, each sent as its own chunk.
    pub fn sse(text: &str) -> Self {
        let chunks = text
            .split("\n\n")
            .filter(|e| !e.trim().is_empty())
            .map(|e| Chunk::Data(format!("{}\n\n", e.trim_matches('\n'))))
            .collect();
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks,
        }
    }

    /// Newline-delimited JSON (Ollama), one chunk per line.
    pub fn ndjson(text: &str) -> Self {
        let chunks = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| Chunk::Data(format!("{l}\n")))
            .collect();
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/x-ndjson".into())],
            chunks,
        }
    }

    /// Load a fixture by extension: `.sse`, `.ndjson`, or `.json` (sent as-is with `status`).
    pub fn fixture(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        Ok(match path.extension().and_then(|e| e.to_str()) {
            Some("sse") => Self::sse(&text),
            Some("ndjson") => Self::ndjson(&text),
            _ => Self {
                status: 200,
                headers: vec![("content-type".into(), "application/json".into())],
                chunks: vec![Chunk::Data(text)],
            },
        })
    }

    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    pub fn with_header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }

    /// Keep only the first `n` data chunks, then stall (simulates a hung stream).
    #[must_use]
    pub fn stall_after(mut self, n: usize) -> Self {
        let mut kept = 0;
        self.chunks.retain(|c| {
            if matches!(c, Chunk::Data(_)) {
                kept += 1;
            }
            kept <= n
        });
        self.chunks.push(Chunk::Stall);
        self
    }

    /// Delay between chunks, for realistic streaming.
    #[must_use]
    pub fn paced(mut self, gap: Duration) -> Self {
        self.chunks = self.chunks.into_iter().flat_map(|c| [Chunk::Delay(gap), c]).collect();
        self
    }
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

#[derive(Default)]
struct Inner {
    queue: VecDeque<Script>,
    /// Served when the queue is empty (the dev server repeats one fixture forever).
    default: Option<Script>,
    requests: Vec<Recorded>,
}

#[derive(Clone)]
pub struct MockProvider {
    addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
}

impl MockProvider {
    /// Bind to an ephemeral localhost port and serve in the background.
    pub async fn start() -> std::io::Result<Self> {
        let inner = Arc::new(Mutex::new(Inner::default()));
        let app = Router::new().fallback(handle).with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { addr, inner })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn push(&self, s: Script) {
        lock(&self.inner).queue.push_back(s);
    }

    pub fn set_default(&self, s: Script) {
        lock(&self.inner).default = Some(s);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        lock(&self.inner).requests.clone()
    }
}

fn lock(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn handle(State(inner): State<Arc<Mutex<Inner>>>, req: Request) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_owned();
    let headers = req
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
        .collect();
    let bytes = axum::body::to_bytes(req.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap_or_default();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    let script = {
        let mut g = lock(&inner);
        g.requests.push(Recorded {
            method,
            path,
            headers,
            body,
        });
        g.queue.pop_front().or_else(|| g.default.clone())
    };
    let Some(script) = script else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "mock-provider: no script queued");
    };
    let stream = futures::stream::iter(script.chunks).then(|c| async move {
        match c {
            Chunk::Data(d) => Some(Ok::<_, std::io::Error>(Bytes::from(d))),
            Chunk::Delay(d) => {
                tokio::time::sleep(d).await;
                None
            }
            Chunk::Stall => {
                futures::future::pending::<()>().await;
                None
            }
        }
    });
    let body = Body::from_stream(stream.filter_map(|x| async move { x }));
    let mut resp = Response::new(body);
    *resp.status_mut() = StatusCode::from_u16(script.status).unwrap_or(StatusCode::OK);
    for (k, v) in script.headers {
        if let (Ok(k), Ok(v)) = (HeaderName::try_from(k), HeaderValue::try_from(v)) {
            resp.headers_mut().insert(k, v);
        }
    }
    resp
}

fn plain(status: StatusCode, msg: &'static str) -> Response {
    let mut r = Response::new(Body::from(msg));
    *r.status_mut() = status;
    r
}
