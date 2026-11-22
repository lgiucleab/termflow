//! Windows: PowerShell 7 with injected integration produces blocks through `ConPTY`.
//! (Runs in CI on windows-latest; cannot be run from the macOS dev machine.)
#![allow(clippy::unwrap_used, clippy::match_wild_err_arm)] // test code

use std::sync::Arc;

use termflow_types::ShellKind;

use super::*;

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
            },
            Box::new(move |b| {
                sink_out.lock().unwrap().extend(b);
                true
            }),
        )
        .unwrap();

    let mut next = async || {
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                match rx.recv().await.unwrap() {
                    AppEvent::BlockFinished { block } => return Ok(block),
                    AppEvent::BlocksDegraded { reason, .. } => return Err(reason),
                    _ => {}
                }
            }
        })
        .await
    };

    mgr.write(id, b"echo hello\r").unwrap();
    let b0 = match next().await {
        Ok(Ok(b)) => b,
        Ok(Err(reason)) => panic!("degraded (ConPTY reordering?): {reason}"),
        Err(_) => panic!("timeout; output:\n{}", String::from_utf8_lossy(&out.lock().unwrap())),
    };
    assert_eq!(b0.command, "echo hello");
    assert_eq!(b0.exit_code, Some(0));

    mgr.write(id, b"cmd /c exit 7\r").unwrap();
    let b1 = next().await.unwrap().unwrap();
    assert_eq!(b1.exit_code, Some(7));
    mgr.close(id).unwrap();
}
