//! Windows: PowerShell 7 with injected integration produces blocks through `ConPTY`.
//! (Runs in CI on windows-latest; cannot be run from the macOS dev machine.)
#![allow(clippy::unwrap_used, clippy::match_wild_err_arm)] // test code

use std::sync::Arc;

use termflow_types::ShellKind;

use super::*;

const DSR: &[u8] = b"\x1b[6n";

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// Waits for the next finished block. Like xterm.js does in the app, answers every cursor
/// position query (`ESC[6n`): `ConPTY`, started with `INHERIT_CURSOR`, sends nothing else until
/// its first query is answered, and `PSReadLine` asks again later.
async fn next_block(
    mgr: &TerminalManager,
    id: TerminalId,
    rx: &mut broadcast::Receiver<AppEvent>,
    out: &Arc<Mutex<Vec<u8>>>,
    answered: &mut usize,
) -> Block {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let asked = count(&out.lock().unwrap(), DSR);
        while *answered < asked {
            mgr.write(id, b"\x1b[1;1R").unwrap();
            *answered += 1;
        }
        tokio::select! {
            ev = rx.recv() => match ev.unwrap() {
                AppEvent::BlockFinished { block } => return block,
                AppEvent::BlocksDegraded { reason, .. } => panic!("degraded (ConPTY reordering?): {reason}"),
                AppEvent::TerminalExited { exit_code, .. } => {
                    panic!("shell exited ({exit_code:?}); output: {:?}", String::from_utf8_lossy(&out.lock().unwrap()))
                }
                _ => {} // block starts and agent events
            },
            () = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        assert!(
            Instant::now() < deadline,
            "timeout; raw output: {:?}",
            String::from_utf8_lossy(&out.lock().unwrap())
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pwsh_blocks() {
    let Some(pwsh) = termflow_pty::find_on_path("pwsh") else {
        eprintln!("skipping: pwsh not installed");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    let (bus, mut rx) = broadcast::channel(256);
    let mgr = TerminalManager::new(store, bus, dir.path().join("si"), true).unwrap();
    let out: Arc<Mutex<Vec<u8>>> = Arc::default();
    let sink_out = out.clone();
    let shell = ShellSpec {
        id: "pwsh".into(),
        name: "pwsh".into(),
        program: pwsh.to_string_lossy().into_owned(),
        args: vec![],
        kind: ShellKind::Pwsh,
    };
    let id = mgr
        .open(
            &OpenOptions {
                shell,
                cwd: Some(dir.path().into()),
                cols: 120,
                rows: 30,
                initial_input: None,
                chat_session: None,
            },
            Box::new(move |b| {
                sink_out.lock().unwrap().extend(b);
                true
            }),
        )
        .unwrap();
    let mut answered = 0;

    mgr.write(id, b"echo hello\r").unwrap();
    let b0 = next_block(&mgr, id, &mut rx, &out, &mut answered).await;
    assert_eq!(b0.command, "echo hello");
    assert_eq!(b0.exit_code, Some(0));

    mgr.write(id, b"cmd /c exit 7\r").unwrap();
    let b1 = next_block(&mgr, id, &mut rx, &out, &mut answered).await;
    assert_eq!(b1.exit_code, Some(7));
    mgr.close(id).unwrap();
}
