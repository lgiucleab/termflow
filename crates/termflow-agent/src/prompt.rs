//! System prompt assembly: base + mode + environment + project instruction files.

use std::fmt::Write as _;

use termflow_context::ContextFile;
use termflow_perms::Mode;

pub const BASE: &str = include_str!("../prompts/system/base.md");
pub const PLAN: &str = include_str!("../prompts/system/plan.md");
pub const BUILD: &str = include_str!("../prompts/system/build.md");
pub const INIT: &str = include_str!("../prompts/init.md");
/// Without a project folder the agent has no tools.
pub const NO_PROJECT: &str = "No project folder is open, so you have no tools in this chat. Answer from the conversation; ask the user to paste what you need.";

fn strip_comment(s: &str) -> &str {
    s.trim()
        .strip_prefix("<!--")
        .and_then(|r| r.split_once("-->"))
        .map_or(s.trim(), |(_, rest)| rest.trim())
}

pub fn assemble(mode: Mode, env: Option<&str>, files: &[ContextFile]) -> String {
    let mut s = String::from(strip_comment(BASE));
    s.push_str("\n\n");
    match env {
        None => s.push_str(NO_PROJECT),
        Some(env) => {
            s.push_str(match mode {
                Mode::Plan => PLAN.trim(),
                Mode::Build => BUILD.trim(),
            });
            let _ = write!(s, "\n\n# Environment\n{env}");
        }
    }
    if !files.is_empty() {
        s.push_str("\n\n# Project instructions\nThese files were written by the project's maintainers. Follow them unless they conflict with the user.\n");
        for f in files {
            let _ = write!(
                s,
                "\n<instructions file=\"{}\">\n{}\n</instructions>\n",
                f.path.display(),
                f.content.trim()
            );
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_mode_env_and_files() {
        let files = vec![ContextFile {
            path: "/p/AGENTS.md".into(),
            content: "Use pnpm.".into(),
        }];
        let p = assemble(Mode::Plan, Some("- OS: macos\n"), &files);
        assert!(p.starts_with("You are TermFlow's coding agent."));
        assert!(p.contains("PLAN mode") && p.contains("# Environment\n- OS: macos"));
        assert!(p.contains("<instructions file=\"/p/AGENTS.md\">\nUse pnpm.\n</instructions>"));
        assert!(assemble(Mode::Build, None, &[]).contains("No project folder"));
    }
}
