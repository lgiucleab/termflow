//! `apply_patch` in the V4A format (owner decision):
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path/new.rs
//! +line
//! *** Update File: path/old.rs
//! *** Move to: path/renamed.rs        (optional)
//! @@ fn anchor_line                   (optional: search after this line)
//!  context
//! -removed
//! +added
//! *** Delete File: path/gone.rs
//! *** End Patch
//! ```
//!
//! No line numbers: hunks are located by their context. The whole patch is validated in memory
//! before anything is written, so it applies completely or not at all.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::fs::unified_diff;
use crate::{CallFacts, Tool, ToolCtx, ToolOutput, arg_str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Chunk {
    pub anchor: Option<String>,
    pub old: Vec<String>,
    pub new: Vec<String>,
    pub at_eof: bool,
}

pub fn parse(text: &str) -> Result<Vec<FileOp>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.trim() == "*** Begin Patch")
        .ok_or("missing `*** Begin Patch`")?;
    let end = lines
        .iter()
        .rposition(|l| l.trim() == "*** End Patch")
        .ok_or("missing `*** End Patch`")?;
    if end <= start {
        return Err("`*** End Patch` before `*** Begin Patch`".into());
    }
    let body = &lines[start + 1..end];
    let mut ops = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = String::new();
            while i < body.len() && !body[i].starts_with("*** ") {
                let l = body[i]
                    .strip_prefix('+')
                    .ok_or_else(|| format!("Add File {path}: every line must start with '+'"))?;
                content.push_str(l);
                content.push('\n');
                i += 1;
            }
            ops.push(FileOp::Add {
                path: path.trim().into(),
                content,
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(FileOp::Delete {
                path: path.trim().into(),
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if let Some(dst) = body.get(i).and_then(|l| l.strip_prefix("*** Move to: ")) {
                move_to = Some(dst.trim().to_owned());
                i += 1;
            }
            let mut chunks = Vec::new();
            let mut cur = Chunk::default();
            while i < body.len() && (!body[i].starts_with("*** ") || body[i].trim() == "*** End of File") {
                let l = body[i];
                if l.trim() == "*** End of File" {
                    cur.at_eof = true;
                } else if let Some(anchor) = l.strip_prefix("@@") {
                    if !cur.old.is_empty() || !cur.new.is_empty() {
                        chunks.push(std::mem::take(&mut cur));
                    }
                    let a = anchor.trim();
                    cur.anchor = (!a.is_empty()).then(|| a.to_owned());
                } else if let Some(r) = l.strip_prefix('-') {
                    cur.old.push(r.to_owned());
                } else if let Some(a) = l.strip_prefix('+') {
                    cur.new.push(a.to_owned());
                } else {
                    // Context line: leading space, or an empty line meaning empty context.
                    let c = l.strip_prefix(' ').unwrap_or(l);
                    cur.old.push(c.to_owned());
                    cur.new.push(c.to_owned());
                }
                i += 1;
            }
            if !cur.old.is_empty() || !cur.new.is_empty() {
                chunks.push(cur);
            }
            if chunks.is_empty() && move_to.is_none() {
                return Err(format!("Update File {path}: no changes"));
            }
            ops.push(FileOp::Update {
                path: path.trim().into(),
                move_to,
                chunks,
            });
        } else if line.trim().is_empty() {
            i += 1;
        } else {
            return Err(format!("unexpected line in patch: {line:?}"));
        }
    }
    if ops.is_empty() {
        return Err("patch contains no file operations".into());
    }
    Ok(ops)
}

/// Find `needle` in `hay` at or after `from`: exact, then ignoring trailing spaces, then trimmed.
fn find(hay: &[String], needle: &[String], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(hay.len()));
    }
    let eq: [fn(&str, &str) -> bool; 3] = [
        |a, b| a == b,
        |a, b| a.trim_end() == b.trim_end(),
        |a, b| a.trim() == b.trim(),
    ];
    eq.iter().find_map(|f| {
        (from..=hay.len().saturating_sub(needle.len()))
            .find(|&s| needle.iter().enumerate().all(|(k, n)| f(&hay[s + k], n)))
    })
}

/// Apply chunks to a file's text.
pub fn apply_chunks(path: &str, text: &str, chunks: &[Chunk]) -> Result<String, String> {
    let had_newline = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut cursor = 0;
    for (n, c) in chunks.iter().enumerate() {
        if let Some(a) = &c.anchor {
            let at = find(&lines, std::slice::from_ref(a), cursor)
                .ok_or_else(|| format!("{path}: anchor `@@ {a}` not found (hunk {})", n + 1))?;
            cursor = at + 1;
        }
        let at = if c.at_eof {
            let s = lines.len().saturating_sub(c.old.len());
            find(&lines, &c.old, s)
        } else {
            find(&lines, &c.old, cursor)
        }
        .ok_or_else(|| {
            format!(
                "{path}: hunk {} does not match the file; re-read the file and resend",
                n + 1
            )
        })?;
        lines.splice(at..at + c.old.len(), c.new.iter().cloned());
        cursor = at + c.new.len();
    }
    let mut out = lines.join("\n");
    if had_newline || text.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

/// Compute every resulting file in memory. `None` content = delete.
pub async fn plan(
    ctx: &ToolCtx,
    ops: &[FileOp],
) -> Result<BTreeMap<PathBuf, (Option<String>, Option<String>)>, String> {
    // path -> (old content, new content)
    let mut out: BTreeMap<PathBuf, (Option<String>, Option<String>)> = BTreeMap::new();
    for op in ops {
        match op {
            FileOp::Add { path, content } => {
                let p = ctx.resolve(path);
                if p.exists() {
                    return Err(format!("Add File {path}: file already exists (use Update File)"));
                }
                out.insert(p, (None, Some(content.clone())));
            }
            FileOp::Delete { path } => {
                let p = ctx.resolve(path);
                let old = tokio::fs::read_to_string(&p)
                    .await
                    .map_err(|e| format!("Delete File {path}: {e}"))?;
                out.insert(p, (Some(old), None));
            }
            FileOp::Update { path, move_to, chunks } => {
                let p = ctx.resolve(path);
                let old = match out.get(&p) {
                    Some((_, Some(pending))) => pending.clone(),
                    _ => tokio::fs::read_to_string(&p)
                        .await
                        .map_err(|e| format!("Update File {path}: {e}"))?,
                };
                let new = apply_chunks(path, &old, chunks)?;
                match move_to {
                    Some(dst) => {
                        out.insert(p, (Some(old), None));
                        out.insert(ctx.resolve(dst), (None, Some(new)));
                    }
                    None => {
                        out.insert(p, (Some(old), Some(new)));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn paths_of(ops: &[FileOp]) -> Vec<String> {
    ops.iter()
        .flat_map(|o| match o {
            FileOp::Add { path, .. } | FileOp::Delete { path } => vec![path.clone()],
            FileOp::Update { path, move_to, .. } => std::iter::once(path.clone()).chain(move_to.clone()).collect(),
        })
        .collect()
}

pub struct ApplyPatch;

#[async_trait::async_trait]
impl Tool for ApplyPatch {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Apply a multi-file patch in V4A format. Start with `*** Begin Patch`, end with `*** End Patch`. \
         Sections: `*** Add File: path` (all lines start with +), `*** Delete File: path`, \
         `*** Update File: path` (optionally followed by `*** Move to: new/path`) with hunks of ' ' context, \
         '-' removed and '+' added lines; separate hunks with `@@`, optionally `@@ <line to search after>`. \
         No line numbers. Include ~3 lines of context. Applies all or nothing."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{"patch":{"type":"string"}},"required":["patch"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let ops = parse(arg_str(args, "patch").unwrap_or_default()).unwrap_or_default();
        let names = paths_of(&ops);
        CallFacts {
            summary: format!(
                "patch {}",
                if names.is_empty() {
                    "(invalid)".into()
                } else {
                    names.join(", ")
                }
            ),
            paths: names.iter().map(|p| ctx.resolve(p)).collect(),
            command: None,
            mutating: true,
        }
    }
    async fn preview(&self, args: &Value, ctx: &ToolCtx) -> Option<String> {
        let ops = match parse(arg_str(args, "patch")?) {
            Ok(o) => o,
            Err(e) => return Some(format!("(this patch will fail: {e})")),
        };
        match plan(ctx, &ops).await {
            Ok(files) => Some(render_diffs(ctx, &files)),
            Err(e) => Some(format!("(this patch will fail: {e})")),
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(text) = arg_str(&args, "patch") else {
            return ToolOutput::err("missing `patch`");
        };
        let ops = match parse(text) {
            Ok(o) => o,
            Err(e) => return ToolOutput::err(format!("invalid patch: {e}")),
        };
        let files = match plan(ctx, &ops).await {
            Ok(f) => f,
            Err(e) => return ToolOutput::err(format!("patch not applied (nothing changed): {e}")),
        };
        for (p, (_, new)) in &files {
            let res = match new {
                Some(content) => {
                    if let Some(parent) = p.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    tokio::fs::write(p, content).await
                }
                None => tokio::fs::remove_file(p).await,
            };
            if let Err(e) = res {
                return ToolOutput::err(format!(
                    "failed writing {} (earlier files may have changed): {e}",
                    p.display()
                ));
            }
        }
        ToolOutput::ok(format!(
            "Applied patch to {} file(s)\n{}",
            files.len(),
            render_diffs(ctx, &files)
        ))
    }
}

fn render_diffs(ctx: &ToolCtx, files: &BTreeMap<PathBuf, (Option<String>, Option<String>)>) -> String {
    files
        .iter()
        .map(|(p, (old, new))| {
            let rel = p.strip_prefix(&ctx.root).unwrap_or(p).display().to_string();
            unified_diff(
                &rel,
                old.as_deref().unwrap_or_default(),
                new.as_deref().unwrap_or_default(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use proptest::prelude::*;

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let d = tempfile::tempdir().unwrap();
        let c = ToolCtx::new(d.path().canonicalize().unwrap());
        (d, c)
    }

    #[tokio::test]
    async fn add_update_move_delete() {
        let (_d, c) = ctx();
        std::fs::write(c.root.join("a.rs"), "fn a() {\n    1\n}\n\nfn b() {\n    1\n}\n").unwrap();
        std::fs::write(c.root.join("gone.txt"), "bye\n").unwrap();
        std::fs::write(c.root.join("old.txt"), "x\ny\n").unwrap();
        let patch = "*** Begin Patch\n*** Add File: new/n.txt\n+hello\n*** Update File: a.rs\n@@ fn b() {\n-    1\n+    2\n*** Update File: old.txt\n*** Move to: moved.txt\n x\n-y\n+z\n*** Delete File: gone.txt\n*** End Patch";
        let o = ApplyPatch.run(json!({"patch":patch}), &c).await;
        assert!(!o.is_error, "{}", o.full);
        assert_eq!(std::fs::read_to_string(c.root.join("new/n.txt")).unwrap(), "hello\n");
        // The anchor selects the second "    1", not the first.
        assert_eq!(
            std::fs::read_to_string(c.root.join("a.rs")).unwrap(),
            "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n"
        );
        assert_eq!(std::fs::read_to_string(c.root.join("moved.txt")).unwrap(), "x\nz\n");
        assert!(!c.root.join("old.txt").exists() && !c.root.join("gone.txt").exists());
    }

    #[tokio::test]
    async fn failing_hunk_changes_nothing() {
        let (_d, c) = ctx();
        std::fs::write(c.root.join("a.txt"), "one\n").unwrap();
        let patch = "*** Begin Patch\n*** Add File: b.txt\n+new\n*** Update File: a.txt\n-nope\n+x\n*** End Patch";
        let o = ApplyPatch.run(json!({"patch":patch}), &c).await;
        assert!(o.is_error && o.full.contains("nothing changed"), "{}", o.full);
        assert!(!c.root.join("b.txt").exists());
        assert_eq!(std::fs::read_to_string(c.root.join("a.txt")).unwrap(), "one\n");
    }

    #[test]
    fn whitespace_tolerant_matching_and_errors() {
        let chunk = Chunk {
            old: vec!["  let x = 1;".into()],
            new: vec!["  let x = 2;".into()],
            ..Chunk::default()
        };
        assert_eq!(
            apply_chunks("f", "let x = 1;   \n", &[chunk]).unwrap(),
            "  let x = 2;\n"
        );
        assert!(parse("no markers").is_err());
        assert!(parse("*** Begin Patch\n*** End Patch").is_err());
        assert!(parse("*** Begin Patch\n*** Add File: x\nmissing plus\n*** End Patch").is_err());
    }

    proptest! {
        /// Replacing any single line of any file via a one-hunk patch yields exactly that change.
        #[test]
        fn single_line_replacement(lines in proptest::collection::vec("[a-z]{1,8}", 1..30), idx in 0usize..30, repl in "[A-Z]{1,8}") {
            let idx = idx % lines.len();
            let text = lines.join("\n") + "\n";
            let lo = idx.saturating_sub(2);
            let hi = (idx + 3).min(lines.len());
            let mut patch = String::from("*** Begin Patch\n*** Update File: f\n");
            for (i, l) in lines.iter().enumerate().take(hi).skip(lo) {
                if i == idx {
                    let _ = write!(patch, "-{l}\n+{repl}\n");
                } else {
                    let _ = writeln!(patch, " {l}");
                }
            }
            patch.push_str("*** End Patch");
            let ops = parse(&patch).unwrap();
            let FileOp::Update { chunks, .. } = &ops[0] else { panic!() };
            let out = apply_chunks("f", &text, chunks).unwrap();
            let mut expect = lines.clone();
            // The first occurrence of the context window is used; it may be an earlier identical one.
            let first = (0..=lines.len() - (hi - lo)).find(|&s| lines[s..s + (hi - lo)] == lines[lo..hi]).unwrap();
            expect[first + (idx - lo)] = repl.clone();
            prop_assert_eq!(out, expect.join("\n") + "\n");
        }

        #[test]
        fn parser_never_panics(s in ".{0,400}") {
            let _ = parse(&format!("*** Begin Patch\n{s}\n*** End Patch"));
        }
    }
}
