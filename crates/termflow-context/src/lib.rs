//! Project context (BUILD_PROMPT §6.5): instruction files from the project up to the git root,
//! a short environment summary, and `@file` expansion. Secrets files are never read automatically.

use std::path::{Path, PathBuf};

use serde::Serialize;

const FILE_CAP: usize = 50_000;
const MENTION_CAP: usize = 100_000;
const NAMES: &[&str] = &["AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextFile {
    pub path: PathBuf,
    pub content: String,
}

/// Never auto-read: `.env*`, `*.pem`, `*.key`, `id_*` (SSH keys).
pub fn is_secret_file(p: &Path) -> bool {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.starts_with(".env") || name.ends_with(".pem") || name.ends_with(".key") || name.starts_with("id_")
}

fn read_capped(p: &Path) -> Option<String> {
    let s = std::fs::read_to_string(p).ok()?;
    Some(if s.len() > FILE_CAP {
        let mut end = FILE_CAP;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n[truncated]", &s[..end])
    } else {
        s
    })
}

/// Instruction files, outermost first: global, then git root … project root, then `.termflow/rules/*.md`.
pub fn discover(root: &Path, global_dir: Option<&Path>) -> Vec<ContextFile> {
    let mut out = Vec::new();
    if let Some(g) = global_dir {
        let p = g.join("AGENTS.md");
        if let Some(content) = read_capped(&p) {
            out.push(ContextFile { path: p, content });
        }
    }
    // Directories from the git root (or the root itself) down to the project root.
    let mut chain: Vec<&Path> = root.ancestors().collect();
    if let Some(git) = chain.iter().position(|d| d.join(".git").exists()) {
        chain.truncate(git + 1);
    } else {
        chain.truncate(1);
    }
    for dir in chain.into_iter().rev() {
        for name in NAMES {
            let p = dir.join(name);
            if let Some(content) = read_capped(&p) {
                out.push(ContextFile { path: p, content });
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root.join(".termflow/rules")) {
        let mut rules: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        rules.sort();
        for p in rules {
            if let Some(content) = read_capped(&p) {
                out.push(ContextFile { path: p, content });
            }
        }
    }
    out
}

/// Short environment description for the system prompt.
pub async fn environment(root: &Path, shell: &str) -> String {
    let date = jiff::Timestamp::now()
        .to_zoned_datetime(jiff::tz::TimeZone::system())
        .date();
    let mut s = format!(
        "- OS: {} ({})\n- Shell for commands: {shell}\n- Project folder: {}\n- Date: {date}\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        root.display()
    );
    let out = tokio::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--branch"])
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .output()
        .await;
    if let Ok(o) = out
        && o.status.success()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        let mut lines = text.lines();
        if let Some(branch) = lines.next() {
            s.push_str(&format!("- Git: {}\n", branch.trim_start_matches("## ")));
        }
        let changed: Vec<&str> = lines.collect();
        if !changed.is_empty() {
            s.push_str(&format!("- Uncommitted changes: {} file(s)", changed.len()));
            let shown: Vec<&str> = changed.iter().take(15).map(|l| l.trim()).collect();
            s.push_str(&format!(
                " ({}{})\n",
                shown.join(", "),
                if changed.len() > 15 { ", …" } else { "" }
            ));
        }
    } else {
        s.push_str("- Git: not a git repository\n");
    }
    s
}

/// Expand `@path` mentions of existing project files into attached file blocks after the message.
/// Secrets files and files outside the project are left as plain text.
pub fn expand_mentions(root: &Path, text: &str) -> String {
    let mut attached = String::new();
    let mut seen = std::collections::HashSet::new();
    for word in text.split_whitespace() {
        let Some(m) = word.strip_prefix('@') else { continue };
        let m = m.trim_end_matches([',', '.', ';', ':', ')', '?', '!']);
        if m.is_empty() || m.starts_with("block:") || !seen.insert(m.to_owned()) {
            continue;
        }
        let p = root.join(m);
        let Ok(canon) = p.canonicalize() else { continue };
        if !canon.starts_with(root) || !canon.is_file() || is_secret_file(&canon) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&canon) else {
            continue;
        };
        let body = if content.len() > MENTION_CAP {
            let mut end = MENTION_CAP;
            while !content.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}\n[truncated; use read for the rest]", &content[..end])
        } else {
            content
        };
        attached.push_str(&format!("\n\n<attached_file path=\"{m}\">\n{body}\n</attached_file>"));
    }
    format!("{text}{attached}")
}

/// Project files for the `@` picker (ignore rules respected, secrets hidden), relative paths.
pub fn project_files(root: &Path, limit: usize) -> Vec<String> {
    let mut w = ignore::WalkBuilder::new(root);
    w.hidden(false)
        .require_git(false)
        .add_custom_ignore_filename(".termflowignore")
        .filter_entry(|e| e.file_name() != ".git");
    w.build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()) && !is_secret_file(e.path()))
        .filter_map(|e| e.path().strip_prefix(root).ok().map(|p| p.display().to_string()))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_files_outermost_first_up_to_git_root() {
        let d = tempfile::tempdir().unwrap();
        let repo = d.path().canonicalize().unwrap().join("repo");
        let proj = repo.join("packages/app");
        std::fs::create_dir_all(proj.join(".termflow/rules")).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(d.path().join("AGENTS.md"), "above git root: ignored").unwrap();
        std::fs::write(repo.join("AGENTS.md"), "repo rules").unwrap();
        std::fs::write(proj.join("CLAUDE.md"), "app rules").unwrap();
        std::fs::write(proj.join(".termflow/rules/style.md"), "style").unwrap();
        let global = d.path().join("global");
        std::fs::create_dir_all(&global).unwrap();
        std::fs::write(global.join("AGENTS.md"), "global").unwrap();
        let got: Vec<String> = discover(&proj, Some(&global)).into_iter().map(|f| f.content).collect();
        assert_eq!(got, ["global", "repo rules", "app rules", "style"]);
    }

    #[test]
    fn mentions_expand_project_files_but_not_secrets_or_outside() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1").unwrap();
        let out = expand_mentions(&root, "look at @src/a.rs, and @.env and @../etc/passwd @missing.rs");
        assert!(out.contains("<attached_file path=\"src/a.rs\">\nfn a() {}"), "{out}");
        assert!(!out.contains("SECRET"));
        assert_eq!(out.matches("<attached_file").count(), 1);
        assert!(project_files(&root, 10).contains(&"src/a.rs".to_owned()));
        assert!(!project_files(&root, 10).contains(&".env".to_owned()));
    }

    #[tokio::test]
    async fn environment_mentions_git_state() {
        let d = tempfile::tempdir().unwrap();
        let env = environment(d.path(), "/bin/bash").await;
        assert!(env.contains("not a git repository") && env.contains("/bin/bash"));
    }
}
