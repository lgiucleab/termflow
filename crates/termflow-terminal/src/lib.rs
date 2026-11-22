//! Terminal sessions: one PTY each, shell-integration blocks, batched output to the UI.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use termflow_pty::{Pty, SpawnRequest};
use termflow_shell_integ::{BlockTracker, Mark, ScanEvent, Scanner, TrackerEvent};
use termflow_store::Store;
use termflow_types::{AgentCli, AppEvent, Block, BlockId, ShellSpec, TerminalId};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Flush output to the UI at most this often (one frame at 120 Hz) ...
const FLUSH_EVERY: Duration = Duration::from_millis(8);
/// ... or as soon as this much is buffered.
const FLUSH_BYTES: usize = 64 * 1024;

/// Agent CLIs offered in "New external agent" when found on PATH (plain-text names only).
const AGENT_CLIS: &[(&str, &str)] = &[
    ("claude", "Claude Code"),
    ("codex", "Codex CLI"),
    ("gemini", "Gemini CLI"),
    ("opencode", "OpenCode"),
    ("copilot", "GitHub Copilot CLI"),
    ("aider", "Aider"),
];

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("no terminal with id {0}")]
    NotFound(TerminalId),
    #[error(transparent)]
    Pty(#[from] termflow_pty::PtyError),
    #[error(transparent)]
    Store(#[from] termflow_store::StoreError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Receives batched output; returns `false` once the receiver is gone.
pub type OutputSink = Box<dyn Fn(Vec<u8>) -> bool + Send + Sync>;

#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub shell: ShellSpec,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    /// Typed into the shell once it shows its first prompt (e.g. `claude\r`).
    pub initial_input: Option<String>,
    /// Chat that owns this shell; its blocks are tagged with it (decision 0002).
    pub chat_session: Option<termflow_types::SessionId>,
}

struct Session {
    pty: Pty,
    paused: watch::Sender<bool>,
    cancel: CancellationToken,
}

pub struct TerminalManager {
    store: Store,
    bus: broadcast::Sender<AppEvent>,
    scripts_dir: PathBuf,
    integration: bool,
    sessions: Arc<Mutex<HashMap<TerminalId, Arc<Session>>>>,
}

impl TerminalManager {
    /// Writes the shell-integration scripts to `scripts_dir`.
    pub fn new(
        store: Store,
        bus: broadcast::Sender<AppEvent>,
        scripts_dir: PathBuf,
        integration: bool,
    ) -> Result<Self, TerminalError> {
        termflow_shell_integ::write_scripts(&scripts_dir)?;
        Ok(Self {
            store,
            bus,
            scripts_dir,
            integration,
            sessions: Arc::default(),
        })
    }

    pub fn shells(&self) -> Vec<ShellSpec> {
        termflow_pty::detect_shells()
    }

    pub fn agents(&self) -> Vec<AgentCli> {
        AGENT_CLIS
            .iter()
            .filter_map(|(id, name)| {
                termflow_pty::find_on_path(id).map(|p| AgentCli {
                    id: (*id).into(),
                    name: (*name).into(),
                    program: p.to_string_lossy().into_owned(),
                })
            })
            .collect()
    }

    /// Spawn a terminal. Must be called inside a tokio runtime.
    pub fn open(&self, opts: &OpenOptions, sink: OutputSink) -> Result<TerminalId, TerminalError> {
        let id = TerminalId::new();
        let injection = if self.integration {
            termflow_shell_integ::inject(opts.shell.kind, &self.scripts_dir, |k| std::env::var(k).ok())
        } else {
            None
        };
        let integrated = injection.is_some();
        let (mut args, env) = injection.map(|i| (i.args, i.env)).unwrap_or_default();
        args.extend(opts.shell.args.iter().cloned());
        let cwd = opts.cwd.clone().or_else(home_dir);

        let (pty, rx) = termflow_pty::spawn(&SpawnRequest {
            program: opts.shell.program.clone(),
            args,
            env,
            cwd,
            cols: opts.cols,
            rows: opts.rows,
            session_id: id.to_string(),
        })?;
        let (paused, paused_rx) = watch::channel(false);
        let session = Arc::new(Session {
            pty,
            paused,
            cancel: CancellationToken::new(),
        });
        self.lock().insert(id, session.clone());

        let initial_input = opts.initial_input.clone();
        if !integrated && let Some(input) = &initial_input {
            session.pty.write(input.as_bytes())?;
        }
        let ctx = Pump {
            id,
            session,
            tracker: BlockTracker::new(id, opts.shell.name.clone()),
            scanner: Scanner::new(),
            initial_input: if integrated { initial_input } else { None },
            chat_session: opts.chat_session,
            store: self.store.clone(),
            bus: self.bus.clone(),
            sink,
            sessions: self.sessions.clone(),
        };
        tokio::spawn(ctx.run(rx, paused_rx));
        tracing::info!(terminal = %id, shell = %opts.shell.id, integrated, "terminal opened");
        Ok(id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<TerminalId, Arc<Session>>> {
        // A poisoned map only means a panic elsewhere; the data is still usable.
        self.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn get(&self, id: TerminalId) -> Result<Arc<Session>, TerminalError> {
        self.lock().get(&id).cloned().ok_or(TerminalError::NotFound(id))
    }

    pub fn write(&self, id: TerminalId, data: &[u8]) -> Result<(), TerminalError> {
        Ok(self.get(id)?.pty.write(data)?)
    }

    pub fn resize(&self, id: TerminalId, cols: u16, rows: u16) -> Result<(), TerminalError> {
        Ok(self.get(id)?.pty.resize(cols, rows)?)
    }

    /// Back-pressure from the UI: while paused we stop reading, and the PTY throttles the child.
    pub fn set_paused(&self, id: TerminalId, paused: bool) -> Result<(), TerminalError> {
        self.get(id)?.paused.send_replace(paused);
        Ok(())
    }

    pub fn close(&self, id: TerminalId) -> Result<(), TerminalError> {
        let s = self.lock().remove(&id).ok_or(TerminalError::NotFound(id))?;
        s.cancel.cancel();
        if let Err(e) = s.pty.kill() {
            tracing::debug!(terminal = %id, error = %e, "kill after exit");
        }
        Ok(())
    }

    pub fn block_output(&self, id: BlockId) -> Result<Option<(Block, String)>, TerminalError> {
        Ok(termflow_store::blocks::get(&self.store.reader()?, id)?)
    }

    pub fn chat_blocks(
        &self,
        chat: termflow_types::SessionId,
        limit: u32,
    ) -> Result<Vec<(Block, String)>, TerminalError> {
        Ok(termflow_store::blocks::for_chat(&self.store.reader()?, chat, limit)?)
    }

    pub fn recent_blocks(&self, limit: u32) -> Result<Vec<Block>, TerminalError> {
        Ok(termflow_store::blocks::recent(&self.store.reader()?, limit)?)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Per-terminal task: reads PTY output, tracks blocks, batches bytes to the sink.
struct Pump {
    id: TerminalId,
    session: Arc<Session>,
    tracker: BlockTracker,
    scanner: Scanner,
    initial_input: Option<String>,
    chat_session: Option<termflow_types::SessionId>,
    store: Store,
    bus: broadcast::Sender<AppEvent>,
    sink: OutputSink,
    sessions: Arc<Mutex<HashMap<TerminalId, Arc<Session>>>>,
}

impl Pump {
    async fn run(mut self, mut rx: mpsc::Receiver<Vec<u8>>, mut paused: watch::Receiver<bool>) {
        let mut buf: Vec<u8> = Vec::with_capacity(FLUSH_BYTES);
        let mut deadline = Instant::now();
        let cancel = self.session.cancel.clone();
        loop {
            let is_paused = *paused.borrow();
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep_until(deadline), if !buf.is_empty() => {
                    if !self.flush(&mut buf) { break; }
                }
                changed = paused.changed() => if changed.is_err() { break },
                chunk = rx.recv(), if !is_paused => {
                    let Some(chunk) = chunk else { break };
                    self.scan(&chunk).await;
                    if buf.is_empty() { deadline = Instant::now() + FLUSH_EVERY; }
                    buf.extend_from_slice(&chunk);
                    if buf.len() >= FLUSH_BYTES && !self.flush(&mut buf) { break; }
                }
            }
        }
        self.flush(&mut buf);

        let session = self.session.clone();
        let exit_code = tokio::task::spawn_blocking(move || session.pty.wait().ok().flatten())
            .await
            .ok()
            .flatten();
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id);
        let _ = self.bus.send(AppEvent::TerminalExited {
            terminal_id: self.id,
            exit_code,
        });
        tracing::info!(terminal = %self.id, ?exit_code, "terminal exited");
    }

    fn flush(&self, buf: &mut Vec<u8>) -> bool {
        if buf.is_empty() {
            return true;
        }
        (self.sink)(std::mem::take(buf))
    }

    async fn scan(&mut self, chunk: &[u8]) {
        for ev in self.scanner.feed(chunk) {
            if matches!(ev, ScanEvent::Mark(Mark::CommandStart))
                && let Some(input) = self.initial_input.take()
                && let Err(e) = self.session.pty.write(input.as_bytes())
            {
                tracing::warn!(terminal = %self.id, error = %e, "initial input failed");
            }
            let Some(tev) = self.tracker.on(ev, jiff::Timestamp::now()) else {
                continue;
            };
            let app_ev = match tev {
                TrackerEvent::Started(mut block) => {
                    block.chat_session_id = self.chat_session;
                    AppEvent::BlockStarted { block }
                }
                TrackerEvent::Finished { mut block, output } => {
                    block.chat_session_id = self.chat_session;
                    let b = block.clone();
                    if let Err(e) = self
                        .store
                        .write(move |c| termflow_store::blocks::insert(c, &b, &output))
                        .await
                    {
                        tracing::warn!(terminal = %self.id, error = %e, "saving block failed");
                    }
                    AppEvent::BlockFinished { block }
                }
                TrackerEvent::Degraded(reason) => {
                    tracing::info!(terminal = %self.id, %reason, "blocks degraded");
                    AppEvent::BlocksDegraded {
                        terminal_id: self.id,
                        reason,
                    }
                }
            };
            let _ = self.bus.send(app_ev); // no subscribers is fine
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod tests_windows;
