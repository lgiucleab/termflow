//! Built-in agent tools (`BUILD_PROMPT` §6.2). Tools never decide permissions — `termflow-perms`
//! does, before `run` is called. Every tool can show what it would change (`preview`) so the
//! approval card displays the exact diff or command.

mod bash;
mod fs;
mod misc;
pub mod patch;
mod search;
mod web;

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use termflow_types::{Block, BlockId};
use tokio_util::sync::CancellationToken;

pub use bash::is_read_only_command;
pub use misc::{Todo, TodoStatus};

/// What the model sees is capped; the full output is stored in the database.
pub const MODEL_OUTPUT_CAP: usize = 30_000;

/// Looks up terminal blocks for `attach_block` (wired to the terminal manager by the core).
pub type BlockLookup = Arc<dyn Fn(Option<BlockId>) -> Option<(Block, String)> + Send + Sync>;

/// Per-call context.
#[derive(Clone)]
pub struct ToolCtx {
    /// Project folder (canonical). Relative paths resolve against it.
    pub root: PathBuf,
    pub cancel: CancellationToken,
    pub todos: Arc<Mutex<Vec<Todo>>>,
    pub blocks: Option<BlockLookup>,
    /// Shell for `bash`: unix default `/bin/bash` (else `/bin/sh`); Windows `pwsh` / `powershell`.
    pub shell: Option<String>,
}

impl ToolCtx {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            cancel: CancellationToken::new(),
            todos: Arc::default(),
            blocks: None,
            shell: None,
        }
    }

    /// Resolve a path argument against the project root (lexically; no symlink following).
    pub fn resolve(&self, p: &str) -> PathBuf {
        normalize(&self.root.join(p))
    }
}

/// Lexically normalize `.` and `..` (works for paths that don't exist yet).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// True if `p` (normalized, absolute) is inside `root`. Symlinked parents are resolved when they exist.
pub fn inside(root: &Path, p: &Path) -> bool {
    let real = |x: &Path| {
        // Canonicalize the deepest existing ancestor so a symlink can't smuggle a path out.
        let mut cur = x.to_path_buf();
        let mut tail = Vec::new();
        loop {
            if let Ok(c) = cur.canonicalize() {
                let mut c = c;
                for t in tail.iter().rev() {
                    c.push(t);
                }
                return c;
            }
            match (cur.file_name().map(std::ffi::OsStr::to_os_string), cur.parent()) {
                (Some(name), Some(parent)) => {
                    tail.push(name);
                    cur = parent.to_path_buf();
                }
                _ => return x.to_path_buf(),
            }
        }
    };
    real(p).starts_with(real(root))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolOutput {
    /// Sent to the model (truncated if needed).
    pub model_text: String,
    /// Stored in the database and shown in the UI.
    pub full: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(text: impl Into<String>) -> Self {
        let full = text.into();
        Self {
            model_text: truncate(&full),
            full,
            is_error: false,
        }
    }

    pub fn err(text: impl Into<String>) -> Self {
        let full = text.into();
        Self {
            model_text: truncate(&full),
            full,
            is_error: true,
        }
    }
}

/// Keep head and tail with an explicit note in between (§6.2).
pub fn truncate(s: &str) -> String {
    if s.len() <= MODEL_OUTPUT_CAP {
        return s.to_owned();
    }
    let half = MODEL_OUTPUT_CAP / 2;
    let mut head_end = half;
    while !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = s.len() - half;
    while !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let omitted = tail_start - head_end;
    format!(
        "{}\n\n[truncated: {omitted} chars omitted; use read with a line range or a narrower search]\n\n{}",
        &s[..head_end],
        &s[tail_start..]
    )
}

/// What permission rules need to know about a call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallFacts {
    /// Resolved paths this call reads or writes.
    pub paths: Vec<PathBuf>,
    pub command: Option<String>,
    pub mutating: bool,
    /// One-line summary for cards ("read src/main.rs", "$ cargo test").
    pub summary: String,
}

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> Value;
    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
    /// Facts for permission checks, from the raw arguments.
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts;
    /// For mutating tools: a diff (or command) shown on the approval card.
    async fn preview(&self, _args: &Value, _ctx: &ToolCtx) -> Option<String> {
        None
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput;
}

pub(crate) fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

pub(crate) fn arg_u64(args: &Value, k: &str) -> Option<u64> {
    args.get(k).and_then(Value::as_u64)
}

/// All built-in tools, in the order they're offered to the model.
pub fn builtin() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(fs::Read),
        Arc::new(search::Grep),
        Arc::new(search::Glob),
        Arc::new(fs::Ls),
        Arc::new(fs::Write),
        Arc::new(fs::Edit),
        Arc::new(patch::ApplyPatch),
        Arc::new(bash::Bash),
        Arc::new(misc::TodoTool),
        Arc::new(misc::AttachBlock),
        Arc::new(web::WebFetch),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_head_and_tail() {
        let s = format!("{}MIDDLE{}", "a".repeat(40_000), "z".repeat(40_000));
        let t = truncate(&s);
        assert!(t.len() < s.len());
        assert!(t.starts_with("aaa") && t.ends_with("zzz"));
        assert!(t.contains("[truncated:"));
        assert_eq!(truncate("short"), "short");
    }

    #[test]
    fn inside_detects_escapes() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        let ctx = ToolCtx::new(root.clone());
        assert!(inside(&root, &ctx.resolve("src/new/file.rs")));
        assert!(!inside(&root, &ctx.resolve("../outside.txt")));
        assert!(!inside(&root, &ctx.resolve("/etc/passwd")));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", root.join("link")).unwrap();
            assert!(!inside(&root, &ctx.resolve("link/x")));
        }
    }
}
