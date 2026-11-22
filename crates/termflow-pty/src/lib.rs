//! PTY spawning (`ConPTY` on Windows via `portable-pty`), I/O, and shell / CLI discovery.

mod detect;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;

pub use detect::{default_shell, detect_shells, find_on_path, shell_kind_for};

/// Reader → consumer queue depth. With 64 KiB reads this caps buffering at ~2 MiB; when the
/// consumer stops pulling, the reader blocks and the child is throttled by the PTY itself.
const QUEUE: usize = 32;
const READ_BUF: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("pty: {0}")]
    Pty(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

fn pty_err(e: impl std::fmt::Display) -> PtyError {
    PtyError::Pty(e.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct SpawnRequest {
    pub program: String,
    pub args: Vec<String>,
    /// Added on top of the inherited environment and the TERM variables.
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    /// Value for `TERMFLOW_SESSION_ID`.
    pub session_id: String,
}

pub struct Pty {
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Spawn `req` in a new PTY. Output arrives on the returned receiver; it closes at EOF.
pub fn spawn(req: &SpawnRequest) -> Result<(Pty, mpsc::Receiver<Vec<u8>>), PtyError> {
    let pair = native_pty_system().openpty(size(req.cols, req.rows)).map_err(pty_err)?;

    let mut cmd = CommandBuilder::new(&req.program);
    cmd.args(&req.args);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "TermFlow");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    cmd.env("TERMFLOW_SESSION_ID", &req.session_id);
    // Apps launched from Finder get no locale; without one, shells and TUIs fall back to ASCII.
    if cfg!(target_os = "macos") && std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "en_US.UTF-8");
    }
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = &req.cwd {
        cmd.cwd(cwd);
    }

    let child = pair.slave.spawn_command(cmd).map_err(pty_err)?;
    drop(pair.slave); // otherwise we never see EOF when the child exits
    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader().map_err(pty_err)?;
    let writer = pair.master.take_writer().map_err(pty_err)?;

    let (tx, rx) = mpsc::channel(QUEUE);
    std::thread::Builder::new()
        .name("termflow-pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; READ_BUF];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break, // EOF, or EIO on Linux/macOS when the child exits
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break; // consumer gone
                        }
                    }
                }
            }
        })?;

    Ok((
        Pty {
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            child: Mutex::new(Some(child)),
        },
        rx,
    ))
}

impl Pty {
    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut w = self.writer.lock().map_err(pty_err)?;
        w.write_all(data)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.master
            .lock()
            .map_err(pty_err)?
            .resize(size(cols, rows))
            .map_err(pty_err)
    }

    pub fn kill(&self) -> Result<(), PtyError> {
        self.killer.lock().map_err(pty_err)?.kill()?;
        Ok(())
    }

    /// Block until the child exits; returns its exit code. Call from a blocking context, once.
    pub fn wait(&self) -> Result<Option<u32>, PtyError> {
        let child = self.child.lock().map_err(pty_err)?.take();
        match child {
            Some(mut c) => Ok(Some(c.wait()?.exit_code())),
            None => Ok(None),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    async fn collect(mut rx: mpsc::Receiver<Vec<u8>>) -> String {
        let mut out = Vec::new();
        while let Some(chunk) = rx.recv().await {
            out.extend(chunk);
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn runs_command_with_env_and_exit_code() {
        let (pty, rx) = spawn(&SpawnRequest {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "printf \"$TERM_PROGRAM:$TERMFLOW_SESSION_ID:$X\"; exit 3".into(),
            ],
            env: vec![("X".into(), "y".into())],
            cols: 80,
            rows: 24,
            session_id: "s1".into(),
            ..Default::default()
        })
        .unwrap();
        let out = collect(rx).await;
        assert!(out.contains("TermFlow:s1:y"), "{out:?}");
        let code = tokio::task::spawn_blocking(move || pty.wait().unwrap()).await.unwrap();
        assert_eq!(code, Some(3));
    }

    #[tokio::test]
    async fn write_and_resize() {
        let (pty, rx) = spawn(&SpawnRequest {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "read l; stty size; echo got:$l".into()],
            cols: 80,
            rows: 24,
            ..Default::default()
        })
        .unwrap();
        pty.resize(100, 30).unwrap();
        pty.write(b"hello\n").unwrap();
        let out = collect(rx).await;
        assert!(out.contains("30 100"), "{out:?}");
        assert!(out.contains("got:hello"), "{out:?}");
    }
}
