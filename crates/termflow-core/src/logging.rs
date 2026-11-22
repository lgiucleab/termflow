//! `tracing` setup: daily rolling file logs with every line passed through the redactor.

use std::io::Write;
use std::path::Path;

use termflow_secrets::Redactor;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, prelude::*};

/// `MakeWriter` that buffers one event, redacts it, then forwards it.
#[derive(Clone)]
pub struct RedactingMakeWriter<M> {
    inner: M,
    redactor: Redactor,
}

impl<M> RedactingMakeWriter<M> {
    pub fn new(inner: M, redactor: Redactor) -> Self {
        Self { inner, redactor }
    }
}

pub struct RedactingWriter<W: Write> {
    buf: Vec<u8>,
    inner: W,
    redactor: Redactor,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let text = String::from_utf8_lossy(&self.buf);
        let res = self.inner.write_all(self.redactor.redact(&text).as_bytes());
        self.buf.clear();
        res.and_then(|()| self.inner.flush())
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush(); // nowhere to report a failed log write
    }
}

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for RedactingMakeWriter<M> {
    type Writer = RedactingWriter<M::Writer>;
    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter {
            buf: Vec::new(),
            inner: self.inner.make_writer(),
            redactor: self.redactor.clone(),
        }
    }
}

/// Install the global subscriber. Keep the guard alive for the app's lifetime.
/// Returns `None` for the guard if a subscriber was already installed (e.g. in tests).
pub fn init(log_dir: &Path, redactor: Redactor) -> Option<WorkerGuard> {
    let appender = tracing_appender::rolling::daily(log_dir, "termflow.log");
    let (writer, guard): (NonBlocking, WorkerGuard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("TERMFLOW_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(writer, redactor));
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .ok()
        .map(|()| guard)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }

    #[test]
    fn log_lines_are_redacted() {
        let sink = Sink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(RedactingMakeWriter::new(sink.clone(), Redactor::new()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(key = "sk-test-0123456789abcdef", "connecting");
        });
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("connecting"));
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("sk-test"));
    }
}
