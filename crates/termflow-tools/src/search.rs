//! grep and glob on ripgrep's crates, respecting .gitignore / .ignore / .termflowignore.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use serde_json::{Value, json};

use crate::{CallFacts, Tool, ToolCtx, ToolOutput, arg_str, arg_u64};

const GREP_DEFAULT_MAX: usize = 200;
const GLOB_MAX: usize = 500;

/// Walker over `dir` honouring ignore files even outside a git repo; skips `.git`.
pub(crate) fn walker(dir: &Path, root: &Path) -> ignore::WalkBuilder {
    let mut w = ignore::WalkBuilder::new(dir);
    w.hidden(false)
        .require_git(false)
        .add_custom_ignore_filename(".termflowignore")
        .filter_entry(|e| e.file_name() != ".git")
        .sort_by_file_name(std::cmp::Ord::cmp);
    // Respect the project's root ignore files even when searching a subfolder.
    for name in [".gitignore", ".ignore", ".termflowignore"] {
        let f = root.join(name);
        if f.is_file() && dir != root {
            let _ = w.add_ignore(f);
        }
    }
    w
}

struct GrepSink<'a> {
    path: &'a str,
    out: &'a mut String,
    count: &'a mut usize,
    max: usize,
}

impl Sink for GrepSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _s: &Searcher, m: &SinkMatch<'_>) -> Result<bool, std::io::Error> {
        *self.count += 1;
        let line = String::from_utf8_lossy(m.bytes());
        let _ = write!(self.out, "{}:{}:{}", self.path, m.line_number().unwrap_or(0), line);
        if !line.ends_with('\n') {
            self.out.push('\n');
        }
        Ok(*self.count < self.max)
    }

    fn context(&mut self, _s: &Searcher, c: &SinkContext<'_>) -> Result<bool, std::io::Error> {
        let line = String::from_utf8_lossy(c.bytes());
        let _ = write!(self.out, "{}-{}-{}", self.path, c.line_number().unwrap_or(0), line);
        if !line.ends_with('\n') {
            self.out.push('\n');
        }
        Ok(true)
    }
}

pub struct Grep;

#[async_trait::async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with a regular expression (ripgrep syntax). Returns path:line:text. Optional glob filter (e.g. \"*.rs\") and context lines."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string"},
            "path":{"type":"string","description":"Directory or file to search (default: project root)"},
            "glob":{"type":"string","description":"Only files matching this glob"},
            "context":{"type":"integer","description":"Lines of context around each match"},
            "case_insensitive":{"type":"boolean"},
            "max_results":{"type":"integer"}
        },"required":["pattern"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or("."));
        CallFacts {
            summary: format!("grep {}", arg_str(args, "pattern").unwrap_or_default()),
            paths: vec![p],
            command: None,
            mutating: false,
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(pattern) = arg_str(&args, "pattern").map(str::to_owned) else {
            return ToolOutput::err("missing `pattern`");
        };
        let dir = ctx.resolve(arg_str(&args, "path").unwrap_or("."));
        let glob = arg_str(&args, "glob").map(str::to_owned);
        let context = arg_u64(&args, "context").map_or(0, |c| usize::try_from(c.min(20)).unwrap_or(0));
        let ci = args.get("case_insensitive").and_then(Value::as_bool).unwrap_or(false);
        let max =
            arg_u64(&args, "max_results").map_or(GREP_DEFAULT_MAX, |m| usize::try_from(m).unwrap_or(GREP_DEFAULT_MAX));
        let root = ctx.root.clone();
        let res = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let matcher = RegexMatcherBuilder::new()
                .case_insensitive(ci)
                .build(&pattern)
                .map_err(|e| e.to_string())?;
            let filter = match glob {
                Some(g) => Some(globset::Glob::new(&g).map_err(|e| e.to_string())?.compile_matcher()),
                None => None,
            };
            let mut searcher = SearcherBuilder::new()
                .line_number(true)
                .before_context(context)
                .after_context(context)
                .build();
            let mut out = String::new();
            let mut count = 0;
            for entry in walker(&dir, &root).build().flatten() {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path()).to_path_buf();
                if let Some(f) = &filter
                    && !f.is_match(&rel)
                    && !rel.file_name().is_some_and(|n| f.is_match(n))
                {
                    continue;
                }
                let rel_s = rel.display().to_string();
                let sink = GrepSink {
                    path: &rel_s,
                    out: &mut out,
                    count: &mut count,
                    max,
                };
                let _ = searcher.search_path(&matcher, entry.path(), sink); // unreadable files are skipped
                if count >= max {
                    let _ = write!(out, "\n[stopped after {max} matches]");
                    break;
                }
            }
            Ok(if out.is_empty() { "No matches.".into() } else { out })
        })
        .await;
        match res {
            Ok(Ok(s)) => ToolOutput::ok(s),
            Ok(Err(e)) => ToolOutput::err(format!("invalid pattern or glob: {e}")),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

pub struct Glob;

#[async_trait::async_trait]
impl Tool for Glob {
    fn name(&self) -> &'static str {
        "glob"
    }
    fn description(&self) -> &'static str {
        "Find files by glob pattern (e.g. \"**/*.rs\", \"src/**/mod.rs\"), newest first. Respects .gitignore."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string"},
            "path":{"type":"string","description":"Directory to search (default: project root)"}
        },"required":["pattern"],"additionalProperties":false})
    }
    fn facts(&self, args: &Value, ctx: &ToolCtx) -> CallFacts {
        let p = ctx.resolve(arg_str(args, "path").unwrap_or("."));
        CallFacts {
            summary: format!("glob {}", arg_str(args, "pattern").unwrap_or_default()),
            paths: vec![p],
            command: None,
            mutating: false,
        }
    }
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(pattern) = arg_str(&args, "pattern").map(str::to_owned) else {
            return ToolOutput::err("missing `pattern`");
        };
        let dir = ctx.resolve(arg_str(&args, "path").unwrap_or("."));
        let root = ctx.root.clone();
        let res = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let m = globset::GlobBuilder::new(&pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| e.to_string())?
                .compile_matcher();
            let mut found: Vec<(std::time::SystemTime, PathBuf)> = walker(&dir, &root)
                .build()
                .flatten()
                .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
                .filter_map(|e| {
                    let rel = e.path().strip_prefix(&dir).ok()?.to_path_buf();
                    m.is_match(&rel).then(|| {
                        let t = e
                            .metadata()
                            .ok()
                            .and_then(|md| md.modified().ok())
                            .unwrap_or(std::time::UNIX_EPOCH);
                        (t, e.path().strip_prefix(&root).unwrap_or(e.path()).to_path_buf())
                    })
                })
                .collect();
            found.sort_by(|a, b| b.0.cmp(&a.0));
            let total = found.len();
            let mut out: String = found
                .iter()
                .take(GLOB_MAX)
                .map(|(_, p)| format!("{}\n", p.display()))
                .collect();
            if total > GLOB_MAX {
                let _ = write!(out, "[{} more not shown]", total - GLOB_MAX);
            }
            Ok(if out.is_empty() {
                "No files matched.".into()
            } else {
                out
            })
        })
        .await;
        match res {
            Ok(Ok(s)) => ToolOutput::ok(s),
            Ok(Err(e)) => ToolOutput::err(format!("invalid glob: {e}")),
            Err(e) => ToolOutput::err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src/sub")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {\n    hello();\n}\n").unwrap();
        std::fs::write(root.join("src/sub/lib.rs"), "pub fn hello() {}\n").unwrap();
        std::fs::write(root.join("target/gen.rs"), "fn hello() {}\n").unwrap();
        std::fs::write(root.join("notes.md"), "Hello world\n").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        (d, ToolCtx::new(root))
    }

    #[tokio::test]
    async fn grep_finds_matches_respecting_ignores() {
        let (_d, c) = ctx();
        let o = Grep.run(json!({"pattern":"hello"}), &c).await;
        assert!(o.full.contains("src/main.rs:2:    hello();"), "{}", o.full);
        assert!(o.full.contains("src/sub/lib.rs:1:"));
        assert!(!o.full.contains("target"));
        assert!(!o.full.contains("notes.md"));
        let ci = Grep
            .run(json!({"pattern":"hello","case_insensitive":true,"glob":"*.md"}), &c)
            .await;
        assert_eq!(ci.full.trim(), "notes.md:1:Hello world");
        let ctxl = Grep
            .run(
                json!({"pattern":"hello\\(\\)","context":1,"glob":"*.rs","path":"src"}),
                &c,
            )
            .await;
        assert!(ctxl.full.contains("src/main.rs-1-fn main() {"), "{}", ctxl.full);
        assert!(Grep.run(json!({"pattern":"("}), &c).await.is_error);
    }

    #[tokio::test]
    async fn glob_matches_relative_paths() {
        let (_d, c) = ctx();
        let o = Glob.run(json!({"pattern":"**/*.rs"}), &c).await;
        let mut lines: Vec<&str> = o.full.lines().collect();
        lines.sort_unstable();
        assert_eq!(lines, ["src/main.rs", "src/sub/lib.rs"]);
        let top = Glob.run(json!({"pattern":"*.md"}), &c).await;
        assert_eq!(top.full.trim(), "notes.md");
    }
}
