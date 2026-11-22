//! Real-shell integration tests: spawn zsh/bash with injected integration and check blocks.
#![allow(clippy::unwrap_used, clippy::match_wild_err_arm)] // test code

use std::sync::Arc;

use termflow_types::ShellKind;

use super::*;

struct Harness {
    mgr: TerminalManager,
    rx: broadcast::Receiver<AppEvent>,
    out: Arc<Mutex<Vec<u8>>>,
    _dir: tempfile::TempDir,
}

fn harness() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    let (bus, rx) = broadcast::channel(256);
    let mgr = TerminalManager::new(store, bus, dir.path().join("si"), true).unwrap();
    Harness {
        mgr,
        rx,
        out: Arc::default(),
        _dir: dir,
    }
}

impl Harness {
    fn open(&self, program: &str, kind: ShellKind, cwd: &std::path::Path) -> TerminalId {
        let out = self.out.clone();
        let shell = ShellSpec {
            id: "t".into(),
            name: "t".into(),
            program: program.into(),
            args: vec![],
            kind,
        };
        self.mgr
            .open(
                &OpenOptions {
                    shell,
                    cwd: Some(cwd.into()),
                    cols: 120,
                    rows: 30,
                    initial_input: None,
                    chat_session: None,
                },
                Box::new(move |b| {
                    out.lock().unwrap().extend(b);
                    true
                }),
            )
            .unwrap()
    }

    async fn next_finished(&mut self) -> Block {
        let fut = async {
            loop {
                match self.rx.recv().await.unwrap() {
                    AppEvent::BlockFinished { block } => return block,
                    AppEvent::BlocksDegraded { reason, .. } => panic!("degraded: {reason}"),
                    _ => {}
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(20), fut).await {
            Ok(b) => b,
            Err(_) => panic!(
                "timeout; output so far:\n{}",
                String::from_utf8_lossy(&self.out.lock().unwrap())
            ),
        }
    }
}

async fn exercise(program: &str, kind: ShellKind) {
    if !std::path::Path::new(program).exists() {
        eprintln!("skipping: {program} not installed");
        return;
    }
    let mut h = harness();
    let work = tempfile::tempdir().unwrap();
    let work_path = work.path().canonicalize().unwrap();
    let id = h.open(program, kind, &work_path);
    h.mgr.write(id, b"echo hello\r").unwrap();
    let b0 = h.next_finished().await;
    assert_eq!(b0.command, "echo hello");
    assert_eq!(b0.exit_code, Some(0));
    assert_eq!(b0.seq, 0);
    assert_eq!(b0.cwd.as_deref(), Some(work_path.to_str().unwrap()));
    assert!(b0.finished_at.unwrap() >= b0.started_at);

    h.mgr.write(id, b"sh -c 'exit 7'\r").unwrap();
    let b1 = h.next_finished().await;
    assert_eq!(b1.exit_code, Some(7));
    assert_eq!(b1.seq, 1);

    h.mgr.write(id, b"cd /\r").unwrap();
    h.next_finished().await;
    h.mgr.write(id, b"true\r").unwrap();
    let b3 = h.next_finished().await;
    assert_eq!(b3.cwd.as_deref(), Some("/"));

    let (_, output) = h.mgr.block_output(b0.id).unwrap().unwrap();
    assert!(output.contains("hello"), "{output:?}");
    h.mgr.close(id).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn zsh_blocks() {
    exercise("/bin/zsh", ShellKind::Zsh).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bash_blocks() {
    exercise("/bin/bash", ShellKind::Bash).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn fish_blocks() {
    let fish = termflow_pty::find_on_path("fish")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    exercise(&fish, ShellKind::Fish).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_emits_event_and_unregisters() {
    let mut h = harness();
    let id = h.open("/bin/sh", ShellKind::Other, std::path::Path::new("/"));
    h.mgr.write(id, b"exit 4\r").unwrap();
    let code = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let AppEvent::TerminalExited { terminal_id, exit_code } = h.rx.recv().await.unwrap()
                && terminal_id == id
            {
                return exit_code;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(code, Some(4));
    assert!(matches!(h.mgr.write(id, b"x"), Err(TerminalError::NotFound(_))));
}
