//! read / write / edit / ls.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Value, json};

use crate::{CallFacts, Tool, ToolCtx, ToolOutput, arg_str, arg_u64};

const READ_DEFAULT_LINES: usize = 2000;
const LS_MAX_ENTRIES: usize = 1000;

fn rel(ctx: &ToolCtx, p: &Path) -> String {
    p.strip_prefix(&ctx.root).unwrap_or(p).display().to_string()
}

/// Unified diff between two texts, for previews and results.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

pub struct Read;

#[async_trait::async_trait]
impl Tool for Read {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read a text file. Returns numbered lines. Use offset (1-based line) and limit to read part of a large file."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"File path, relative to the project root or absolute"},
            "offset":{"type":"integer","description":"First line to return (1-based)"},
            "limit":{"type":"integer","description":"Maximum number of lines (default 2000)"}
        },"required":["path"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or_default());
        CallFacts {
            summary: format!("read {}", rel(ctx, &p)),
            paths: vec![p],
            command: None,
            mutating: false,
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(path) = arg_str(&args, "path") else {
            return ToolOutput::err("missing `path`");
        };
        let p = ctx.resolve(path);
        let bytes = match tokio::fs::read(&p).await {
            Ok(b) => b,
            Err(e) => return ToolOutput::err(format!("cannot read {}: {e}", rel(ctx, &p))),
        };
        if bytes.iter().take(8192).any(|b| *b == 0) {
            return ToolOutput::err(format!(
                "{} looks like a binary file ({} bytes); not shown",
                rel(ctx, &p),
                bytes.len()
            ));
        }
        let text = String::from_utf8_lossy(&bytes);
        let total = text.lines().count();
        let start = arg_u64(&args, "offset").map_or(1, |o| usize::try_from(o.max(1)).unwrap_or(1));
        let limit =
            arg_u64(&args, "limit").map_or(READ_DEFAULT_LINES, |l| usize::try_from(l).unwrap_or(READ_DEFAULT_LINES));
        let mut out = String::new();
        for (i, line) in text.lines().enumerate().skip(start - 1).take(limit) {
            let _ = writeln!(out, "{:>6}\t{line}", i + 1);
        }
        let shown_end = (start - 1 + limit).min(total);
        if shown_end < total || start > 1 {
            let _ = write!(out, "\n[lines {start}-{shown_end} of {total}]");
        }
        if out.is_empty() {
            out = "(empty file)".into();
        }
        ToolOutput::ok(out)
    }
}

pub struct Write;

#[async_trait::async_trait]
impl Tool for Write {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Create a file or replace its whole content. Prefer `edit` or `apply_patch` for changes to existing files."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "content":{"type":"string","description":"The complete new file content"}
        },"required":["path","content"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or_default());
        CallFacts {
            summary: format!("write {}", rel(ctx, &p)),
            paths: vec![p],
            command: None,
            mutating: true,
        }
    }
    async fn preview(&self, args: &Value, ctx: &ToolCtx) -> Option<String> {
        let p = ctx.resolve(arg_str(args, "path")?);
        let old = tokio::fs::read_to_string(&p).await.unwrap_or_default();
        Some(unified_diff(&rel(ctx, &p), &old, arg_str(args, "content")?))
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let (Some(path), Some(content)) = (arg_str(&args, "path"), arg_str(&args, "content")) else {
            return ToolOutput::err("missing `path` or `content`");
        };
        let p = ctx.resolve(path);
        let old = tokio::fs::read_to_string(&p).await.ok();
        if let Some(parent) = p.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolOutput::err(format!("cannot create {}: {e}", parent.display()));
        }
        if let Err(e) = tokio::fs::write(&p, content).await {
            return ToolOutput::err(format!("cannot write {}: {e}", rel(ctx, &p)));
        }
        let verb = if old.is_some() { "Updated" } else { "Created" };
        ToolOutput::ok(format!(
            "{verb} {} ({} bytes)\n{}",
            rel(ctx, &p),
            content.len(),
            unified_diff(&rel(ctx, &p), old.as_deref().unwrap_or_default(), content)
        ))
    }
}

pub struct Edit;

/// Apply an exact replacement; errors when the match is missing or ambiguous.
pub fn apply_edit(text: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("`old_string` is empty".into());
    }
    let n = text.matches(old).count();
    match (n, all) {
        (0, _) => Err("`old_string` not found in the file; read it again and copy the exact text".into()),
        (1, _) | (_, true) => Ok(text.replace(old, new)),
        (n, false) => Err(format!(
            "`old_string` occurs {n} times; add surrounding lines to make it unique or set replace_all"
        )),
    }
}

#[async_trait::async_trait]
impl Tool for Edit {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        "Replace an exact string in a file. `old_string` must match exactly once (including whitespace) unless replace_all is true."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "old_string":{"type":"string"},
            "new_string":{"type":"string"},
            "replace_all":{"type":"boolean"}
        },"required":["path","old_string","new_string"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or_default());
        CallFacts {
            summary: format!("edit {}", rel(ctx, &p)),
            paths: vec![p],
            command: None,
            mutating: true,
        }
    }
    async fn preview(&self, args: &Value, ctx: &ToolCtx) -> Option<String> {
        let p = ctx.resolve(arg_str(args, "path")?);
        let old = tokio::fs::read_to_string(&p).await.ok()?;
        let all = args.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
        match apply_edit(&old, arg_str(args, "old_string")?, arg_str(args, "new_string")?, all) {
            Ok(new) => Some(unified_diff(&rel(ctx, &p), &old, &new)),
            Err(e) => Some(format!("(this edit will fail: {e})")),
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let (Some(path), Some(old_s), Some(new_s)) = (
            arg_str(&args, "path"),
            arg_str(&args, "old_string"),
            arg_str(&args, "new_string"),
        ) else {
            return ToolOutput::err("missing `path`, `old_string` or `new_string`");
        };
        let p = ctx.resolve(path);
        let old = match tokio::fs::read_to_string(&p).await {
            Ok(t) => t,
            Err(e) => return ToolOutput::err(format!("cannot read {}: {e}", rel(ctx, &p))),
        };
        let all = args.get("replace_all").and_then(Value::as_bool).unwrap_or(false);
        let new = match apply_edit(&old, old_s, new_s, all) {
            Ok(n) => n,
            Err(e) => return ToolOutput::err(e),
        };
        if let Err(e) = tokio::fs::write(&p, &new).await {
            return ToolOutput::err(format!("cannot write {}: {e}", rel(ctx, &p)));
        }
        ToolOutput::ok(format!(
            "Edited {}\n{}",
            rel(ctx, &p),
            unified_diff(&rel(ctx, &p), &old, &new)
        ))
    }
}

pub struct Ls;

#[async_trait::async_trait]
impl Tool for Ls {
    fn name(&self) -> &'static str {
        "ls"
    }
    fn description(&self) -> &'static str {
        "List a directory as a tree (respects .gitignore). depth defaults to 2."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"Directory (default: project root)"},
            "depth":{"type":"integer"}
        },"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or("."));
        CallFacts {
            summary: format!("ls {}", rel(ctx, &p)),
            paths: vec![p],
            command: None,
            mutating: false,
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let dir = ctx.resolve(arg_str(&args, "path").unwrap_or("."));
        let depth = arg_u64(&args, "depth").map_or(2, |d| usize::try_from(d).unwrap_or(2));
        let root = ctx.root.clone();
        let res = tokio::task::spawn_blocking(move || {
            let mut out = String::new();
            let mut n = 0;
            for entry in crate::search::walker(&dir, &root)
                .max_depth(Some(depth))
                .build()
                .flatten()
            {
                if entry.depth() == 0 {
                    continue;
                }
                n += 1;
                if n > LS_MAX_ENTRIES {
                    let _ = writeln!(out, "… (more entries not shown)");
                    break;
                }
                let name = entry.file_name().to_string_lossy();
                let slash = if entry.file_type().is_some_and(|t| t.is_dir()) {
                    "/"
                } else {
                    ""
                };
                let _ = writeln!(out, "{}{name}{slash}", "  ".repeat(entry.depth() - 1));
            }
            if out.is_empty() { "(empty)".to_owned() } else { out }
        })
        .await;
        match res {
            Ok(s) => ToolOutput::ok(s),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let d = tempfile::tempdir().unwrap();
        let c = ToolCtx::new(d.path().canonicalize().unwrap());
        (d, c)
    }

    #[tokio::test]
    async fn read_numbers_lines_and_ranges() {
        let (_d, c) = ctx();
        std::fs::write(c.root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let o = Read.run(json!({"path":"a.txt"}), &c).await;
        assert_eq!(o.full, "     1\tone\n     2\ttwo\n     3\tthree\n");
        let o = Read.run(json!({"path":"a.txt","offset":2,"limit":1}), &c).await;
        assert!(o.full.starts_with("     2\ttwo\n") && o.full.contains("[lines 2-2 of 3]"));
        std::fs::write(c.root.join("b.bin"), [0u8, 1, 2]).unwrap();
        assert!(Read.run(json!({"path":"b.bin"}), &c).await.is_error);
        assert!(Read.run(json!({"path":"missing"}), &c).await.is_error);
    }

    #[tokio::test]
    async fn write_creates_dirs_and_reports_diff() {
        let (_d, c) = ctx();
        let o = Write
            .run(json!({"path":"src/new.rs","content":"fn a() {}\n"}), &c)
            .await;
        assert!(!o.is_error, "{}", o.full);
        assert!(o.full.starts_with("Created src/new.rs"));
        assert_eq!(
            std::fs::read_to_string(c.root.join("src/new.rs")).unwrap(),
            "fn a() {}\n"
        );
        let pv = Write
            .preview(&json!({"path":"src/new.rs","content":"fn b() {}\n"}), &c)
            .await
            .unwrap();
        assert!(pv.contains("-fn a() {}") && pv.contains("+fn b() {}"));
    }

    #[tokio::test]
    async fn edit_requires_unique_match() {
        let (_d, c) = ctx();
        std::fs::write(c.root.join("x.rs"), "let a = 1;\nlet a = 1;\nlet b = 2;\n").unwrap();
        let dup = Edit
            .run(
                json!({"path":"x.rs","old_string":"let a = 1;","new_string":"let a = 3;"}),
                &c,
            )
            .await;
        assert!(dup.is_error && dup.full.contains("2 times"));
        let none = Edit
            .run(json!({"path":"x.rs","old_string":"nope","new_string":"x"}), &c)
            .await;
        assert!(none.is_error && none.full.contains("not found"));
        let ok = Edit
            .run(
                json!({"path":"x.rs","old_string":"let b = 2;","new_string":"let b = 5;"}),
                &c,
            )
            .await;
        assert!(!ok.is_error);
        let all = Edit
            .run(
                json!({"path":"x.rs","old_string":"let a = 1;","new_string":"let a = 9;","replace_all":true}),
                &c,
            )
            .await;
        assert!(!all.is_error);
        assert_eq!(
            std::fs::read_to_string(c.root.join("x.rs")).unwrap(),
            "let a = 9;\nlet a = 9;\nlet b = 5;\n"
        );
    }

    #[tokio::test]
    async fn ls_respects_gitignore() {
        let (_d, c) = ctx();
        std::fs::create_dir_all(c.root.join("src")).unwrap();
        std::fs::create_dir_all(c.root.join("target/debug")).unwrap();
        std::fs::write(c.root.join("src/main.rs"), "").unwrap();
        std::fs::write(c.root.join(".gitignore"), "target/\n").unwrap();
        let o = Ls.run(json!({}), &c).await;
        assert!(o.full.contains("src/") && o.full.contains("  main.rs"), "{}", o.full);
        assert!(!o.full.contains("target"), "{}", o.full);
    }
}
