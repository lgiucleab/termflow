//! `bash`: one non-interactive command in its own process (never the user's visible terminal),
//! cwd = project root, with a timeout and an output cap. On timeout or Esc the whole process
//! group (Unix) / process tree (Windows) is killed.

use std::fmt::Write as _;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::{CallFacts, Tool, ToolCtx, ToolOutput, arg_str, arg_u64};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_TIMEOUT: Duration = Duration::from_secs(600);
/// Bytes kept from a command's output (head + tail) for the DB; the model gets less.
const KEEP: usize = 1024 * 1024;

/// Commands that only read (`BUILD_PROMPT` §6.4 allowlist). Anything with shell control operators,
/// redirection or substitution is never considered read-only.
const READ_ONLY_PREFIXES: &[&str] = &[
    "git status",
    "git diff",
    "git log",
    "git show",
    "git branch",
    "ls",
    "pwd",
    "cat ",
    "head ",
    "tail ",
    "wc ",
    "cargo check",
    "cargo test",
    "cargo clippy",
    "cargo tree",
    "cargo metadata",
    "pnpm test",
    "npm test",
    "rg ",
    "which ",
];

pub fn is_read_only_command(cmd: &str) -> bool {
    let c = cmd.trim();
    let dangerous = [";", "&", "|", ">", "<", "`", "$(", "\n"];
    !c.is_empty()
        && !dangerous.iter().any(|d| c.contains(d))
        && READ_ONLY_PREFIXES.iter().any(|p| {
            c == p.trim_end()
                || c.starts_with(p.trim_end()) && {
                    let rest = &c[p.trim_end().len()..];
                    rest.is_empty() || rest.starts_with(' ')
                }
        })
}

pub struct Bash;

fn shell(ctx: &ToolCtx) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let sh = ctx.shell.clone().unwrap_or_else(|| {
            if which("pwsh") {
                "pwsh".into()
            } else {
                "powershell".into()
            }
        });
        (
            sh,
            vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into()],
        )
    }
    #[cfg(not(windows))]
    {
        let sh = ctx.shell.clone().unwrap_or_else(|| {
            if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash".into()
            } else {
                "/bin/sh".into()
            }
        });
        (sh, vec!["-c".into()])
    }
}

#[cfg(windows)]
fn which(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(format!("{name}.exe")).is_file()))
}

/// Kill the command and everything it started.
fn kill_tree(pid: u32) {
    // Spawning the OS tool avoids `unsafe` signal calls (workspace forbids unsafe code).
    #[cfg(unix)]
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .status();
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .status();
}

/// Keeps the first and last KEEP/2 bytes of a stream.
#[derive(Default)]
struct Capped {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    dropped: usize,
}

impl Capped {
    fn push(&mut self, b: &[u8]) {
        let room = (KEEP / 2).saturating_sub(self.head.len());
        let (h, t) = b.split_at(room.min(b.len()));
        self.head.extend_from_slice(h);
        self.tail.extend(t);
        while self.tail.len() > KEEP / 2 {
            self.tail.pop_front();
            self.dropped += 1;
        }
    }
    fn text(&self) -> String {
        let mut s = String::from_utf8_lossy(&self.head).into_owned();
        if self.dropped > 0 {
            let _ = write!(s, "\n[… {} bytes of output omitted …]\n", self.dropped);
        }
        s.push_str(&String::from_utf8_lossy(&self.tail.iter().copied().collect::<Vec<_>>()));
        s
    }
}

#[async_trait::async_trait]
impl Tool for Bash {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in the project folder (non-interactive; no TTY). Returns exit code and combined output. \
         Default timeout 120 s (timeout_ms up to 600000). Don't start servers or interactive programs."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "command":{"type":"string"},
            "timeout_ms":{"type":"integer"}
        },"required":["command"],"additionalProperties":false})
    }
    fn timeout(&self) -> Duration {
        MAX_TIMEOUT
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let cmd = arg_str(args, "command").unwrap_or_default().to_owned();
        CallFacts {
            summary: format!("$ {}", cmd.lines().next().unwrap_or_default()),
            mutating: !is_read_only_command(&cmd),
            paths: vec![ctx.root.clone()],
            command: Some(cmd),
        }
    }
    async fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Option<String> {
        arg_str(args, "command").map(|c| format!("$ {c}"))
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(cmd) = arg_str(&args, "command") else {
            return ToolOutput::err("missing `command`");
        };
        let timeout =
            arg_u64(&args, "timeout_ms").map_or(DEFAULT_TIMEOUT, |ms| Duration::from_millis(ms).min(MAX_TIMEOUT));
        let (sh, mut sh_args) = shell(ctx);
        // Merge stderr into stdout so output stays in order.
        #[cfg(not(windows))]
        sh_args.push(format!("exec 2>&1\n{cmd}"));
        #[cfg(windows)]
        sh_args.push(format!("& {{ {cmd} }} 2>&1"));
        let mut c = tokio::process::Command::new(&sh);
        c.args(&sh_args)
            .current_dir(&ctx.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env("PAGER", "cat")
            .env("GIT_PAGER", "cat")
            .env("GIT_TERMINAL_PROMPT", "0")
            .kill_on_drop(true);
        #[cfg(unix)]
        c.process_group(0);
        let started = Instant::now();
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => return ToolOutput::err(format!("could not start {sh}: {e}")),
        };
        let pid = child.id().unwrap_or(0);
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let mut buf = Capped::default();
        let mut ebuf = Capped::default();
        let mut chunk = vec![0u8; 16 * 1024];
        let mut echunk = vec![0u8; 4 * 1024];
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        let status = loop {
            tokio::select! {
                n = async { match out.as_mut() { Some(o) => o.read(&mut chunk).await, None => std::future::pending().await } } => match n {
                    Ok(0) | Err(_) => out = None,
                    Ok(n) => buf.push(&chunk[..n]),
                },
                n = async { match err.as_mut() { Some(e) => e.read(&mut echunk).await, None => std::future::pending().await } } => match n {
                    Ok(0) | Err(_) => err = None,
                    Ok(n) => ebuf.push(&echunk[..n]),
                },
                s = child.wait(), if out.is_none() && err.is_none() => break s.ok(),
                () = &mut deadline => {
                    kill_tree(pid);
                    let _ = child.kill().await;
                    return ToolOutput::err(format!("timed out after {}s (killed)\n{}", timeout.as_secs(), buf.text()));
                }
                () = ctx.cancel.cancelled() => {
                    kill_tree(pid);
                    let _ = child.kill().await;
                    return ToolOutput::err(format!("cancelled by user (killed)\n{}", buf.text()));
                }
            }
        };
        let mut text = buf.text();
        let etext = ebuf.text();
        if !etext.is_empty() {
            text.push_str(&etext);
        }
        let code = status.and_then(|s| s.code());
        let secs = started.elapsed().as_secs_f32();
        let header = match code {
            Some(c) => format!("exit code {c} ({secs:.1}s)"),
            None => format!("terminated by signal ({secs:.1}s)"),
        };
        let body = if text.trim().is_empty() {
            "(no output)".to_owned()
        } else {
            text
        };
        let o = format!("{header}\n{body}");
        if code == Some(0) {
            ToolOutput::ok(o)
        } else {
            ToolOutput::err(o)
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)] // test code
mod tests {
    use super::*;

    #[test]
    fn read_only_allowlist() {
        for ok in [
            "git status",
            "git diff --stat",
            "ls -la",
            "cargo test",
            "cargo test -p x",
            "cat README.md",
        ] {
            assert!(is_read_only_command(ok), "{ok}");
        }
        for bad in [
            "git status && rm -rf x",
            "ls; rm x",
            "cat a > b",
            "cargo testx",
            "rm -rf /",
            "git push",
            "echo $(whoami)",
            "lsblk",
            "ls | sh",
            "",
        ] {
            assert!(!is_read_only_command(bad), "{bad}");
        }
    }

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let d = tempfile::tempdir().unwrap();
        let c = ToolCtx::new(d.path().canonicalize().unwrap());
        (d, c)
    }

    #[tokio::test]
    async fn runs_in_root_with_exit_code_and_merged_output() {
        let (_d, c) = ctx();
        let o = Bash
            .run(json!({"command":"pwd; echo out; echo err 1>&2; exit 3"}), &c)
            .await;
        assert!(o.is_error);
        assert!(o.full.starts_with("exit code 3"), "{}", o.full);
        assert!(o.full.contains(&c.root.display().to_string()));
        assert!(o.full.contains("out\nerr"), "{}", o.full);
        let ok = Bash.run(json!({"command":"true"}), &c).await;
        assert!(!ok.is_error && ok.full.contains("(no output)"));
    }

    #[tokio::test]
    async fn timeout_kills_process_group() {
        let (_d, c) = ctx();
        let t = Instant::now();
        let o = Bash
            .run(json!({"command":"sleep 30 & sleep 30","timeout_ms":300}), &c)
            .await;
        assert!(o.is_error && o.full.contains("timed out"));
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancel_kills() {
        let (_d, c) = ctx();
        let cancel = c.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            cancel.cancel();
        });
        let o = Bash.run(json!({"command":"sleep 30"}), &c).await;
        assert!(o.full.contains("cancelled"));
    }

    #[tokio::test]
    async fn large_output_is_capped() {
        let (_d, c) = ctx();
        let o = Bash.run(json!({"command":"yes x | head -c 3000000"}), &c).await;
        assert!(o.full.contains("bytes of output omitted"));
        assert!(o.full.len() < 1_200_000);
        assert!(o.model_text.len() <= crate::MODEL_OUTPUT_CAP + 200);
    }
}
