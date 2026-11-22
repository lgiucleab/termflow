//! Permission rules (`BUILD_PROMPT` §6.4): ordered, **last matching rule wins**, effects
//! allow / ask / deny. A configured `deny` can never be overridden by a session grant.
//! Plan mode rejects mutating calls here, in the perms layer, not just in the prompt.

use std::collections::HashSet;
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use termflow_tools::{CallFacts, inside, is_read_only_command};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Ask,
    Deny,
}

/// One `[[permissions]]` entry. Unset matchers match anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Tool name(s) or globs (`"*"`, `"write"`, `["write","edit"]`).
    #[serde(default, deserialize_with = "one_or_many")]
    #[schemars(with = "Vec<String>")]
    pub tool: Vec<String>,
    /// Bash command globs (`"git status*"`). For allow: every `;`/`&&`/`|` segment must match.
    #[serde(default, deserialize_with = "one_or_many")]
    #[schemars(with = "Vec<String>")]
    pub command: Vec<String>,
    /// Path globs, matched against the absolute path (`"**/.env*"`).
    #[serde(default, deserialize_with = "one_or_many")]
    #[schemars(with = "Vec<String>")]
    pub path: Vec<String>,
    /// Matches when any path is outside the project folder.
    #[serde(default)]
    pub path_outside_workspace: bool,
    pub effect: Effect,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

fn rule(tool: &[&str], effect: Effect) -> Rule {
    Rule {
        tool: tool.iter().map(|s| (*s).to_owned()).collect(),
        command: vec![],
        path: vec![],
        path_outside_workspace: false,
        effect,
    }
}

/// The "Balanced" defaults from §6.4 (plus: reading obvious secrets files asks).
pub fn default_rules() -> Vec<Rule> {
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    vec![
        rule(&["*"], Effect::Ask),
        rule(&["read", "grep", "glob", "ls", "todo", "attach_block"], Effect::Allow),
        Rule {
            command: s(&[
                "git status*",
                "git diff*",
                "git log*",
                "git show*",
                "ls*",
                "pwd",
                "cat *",
                "cargo check*",
                "cargo test*",
                "cargo clippy*",
                "pnpm test*",
                "npm test*",
            ]),
            ..rule(&["bash"], Effect::Allow)
        },
        Rule {
            path: s(&["**/.env*", "**/*.pem", "**/id_*", "**/*.key"]),
            ..rule(&["read"], Effect::Ask)
        },
        Rule {
            command: s(&[
                "rm -rf /*",
                "rm -rf ~*",
                "sudo *",
                "git push --force*",
                "git push -f*",
                "curl * | sh*",
                "curl * | bash*",
                "wget * | sh*",
                "*Remove-Item -Recurse -Force C:\\*",
                "mkfs*",
                "dd if=*",
            ]),
            ..rule(&["bash"], Effect::Deny)
        },
        Rule {
            path: s(&["**/.env*", "**/*.pem", "**/id_*", "**/*.key"]),
            ..rule(&["write", "edit", "apply_patch"], Effect::Deny)
        },
        Rule {
            path_outside_workspace: true,
            ..rule(&["write", "edit", "apply_patch"], Effect::Deny)
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Build,
    /// Read-only: mutating calls are denied here.
    Plan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny(String),
}

fn glob(p: &str) -> Option<GlobMatcher> {
    GlobBuilder::new(p)
        .literal_separator(false)
        .build()
        .ok()
        .map(|g| g.compile_matcher())
}

fn any_glob(patterns: &[String], s: &str) -> bool {
    patterns.iter().filter_map(|p| glob(p)).any(|g| g.is_match(s))
}

/// Split a shell command into the segments joined by `;`, `&&`, `||`, `|`, newlines.
fn segments(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ';' | '\n' | '|' | '&' => {
                if matches!((c, chars.peek()), ('|' | '&', Some('|' | '&'))) {
                    chars.next();
                }
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_owned());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

impl Rule {
    fn matches(&self, tool: &str, f: &CallFacts, root: &Path) -> bool {
        if !self.tool.is_empty()
            && !self
                .tool
                .iter()
                .any(|t| t == "*" || t == tool || glob(t).is_some_and(|g| g.is_match(tool)))
        {
            return false;
        }
        if !self.command.is_empty() {
            let Some(cmd) = &f.command else { return false };
            let segs = segments(cmd);
            let ok = if self.effect == Effect::Deny {
                // Deny if the whole command or any segment matches.
                any_glob(&self.command, cmd.trim()) || segs.iter().any(|s| any_glob(&self.command, s))
            } else {
                // Allow/ask only when every segment matches and there's no redirection/substitution.
                let unsafe_chars = [">", "<", "`", "$("];
                !segs.is_empty()
                    && !unsafe_chars.iter().any(|u| cmd.contains(u))
                    && segs.iter().all(|s| any_glob(&self.command, s))
            };
            if !ok {
                return false;
            }
        }
        if !self.path.is_empty() && !f.paths.iter().any(|p| any_glob(&self.path, &p.to_string_lossy())) {
            return false;
        }
        if self.path_outside_workspace && !f.paths.iter().any(|p| !inside(root, p)) {
            return false;
        }
        true
    }
}

/// Grants made with "Allow for this session".
#[derive(Debug, Default, Clone)]
pub struct SessionGrants(HashSet<String>);

impl SessionGrants {
    /// Key: tool + command (bash) or tool alone. Granting `bash` for one command doesn't grant others.
    fn key(tool: &str, f: &CallFacts) -> String {
        match &f.command {
            Some(c) => format!("{tool}\u{0}{}", c.trim()),
            None => tool.to_owned(),
        }
    }
    pub fn grant(&mut self, tool: &str, f: &CallFacts) {
        self.0.insert(Self::key(tool, f));
    }
    pub fn has(&self, tool: &str, f: &CallFacts) -> bool {
        self.0.contains(&Self::key(tool, f))
    }
}

/// Decide a call. Order: plan mode → rules (last match) → deny wins over grants → grants → result.
pub fn evaluate(
    rules: &[Rule],
    grants: &SessionGrants,
    mode: Mode,
    tool: &str,
    f: &CallFacts,
    root: &Path,
) -> Decision {
    if mode == Mode::Plan && f.mutating {
        let what = f
            .command
            .as_deref()
            .map_or_else(|| tool.to_owned(), |c| format!("`{c}`"));
        return Decision::Deny(format!(
            "Plan mode is read-only: {what} was not run. Finish the plan; the user can switch to Build mode to execute it."
        ));
    }
    let last = rules
        .iter()
        .rev()
        .find(|r| r.matches(tool, f, root))
        .map_or(Effect::Ask, |r| r.effect);
    if last == Effect::Deny {
        return Decision::Deny(format!("blocked by a permission rule ({tool}: {})", f.summary));
    }
    if grants.has(tool, f) {
        return Decision::Allow;
    }
    match last {
        Effect::Allow => Decision::Allow,
        Effect::Ask | Effect::Deny => Decision::Ask,
    }
}

/// Plan mode allows bash only for allowlisted read-only commands.
pub fn bash_is_mutating(cmd: &str) -> bool {
    !is_read_only_command(cmd)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn facts(paths: &[&str], cmd: Option<&str>, mutating: bool) -> CallFacts {
        CallFacts {
            paths: paths.iter().map(PathBuf::from).collect(),
            command: cmd.map(Into::into),
            mutating,
            summary: String::new(),
        }
    }

    const ROOT: &str = "/work/proj";

    fn eval(tool: &str, f: &CallFacts) -> Decision {
        evaluate(
            &default_rules(),
            &SessionGrants::default(),
            Mode::Build,
            tool,
            f,
            Path::new(ROOT),
        )
    }

    #[test]
    fn balanced_defaults() {
        assert_eq!(
            eval("read", &facts(&["/work/proj/src/a.rs"], None, false)),
            Decision::Allow
        );
        assert_eq!(eval("grep", &facts(&["/work/proj"], None, false)), Decision::Allow);
        assert_eq!(
            eval("write", &facts(&["/work/proj/src/a.rs"], None, true)),
            Decision::Ask
        );
        assert_eq!(
            eval("edit", &facts(&["/work/proj/src/a.rs"], None, true)),
            Decision::Ask
        );
        assert_eq!(eval("webfetch", &facts(&[], None, false)), Decision::Ask);
        assert_eq!(
            eval("bash", &facts(&[ROOT], Some("cargo test -p x"), false)),
            Decision::Allow
        );
        assert_eq!(
            eval("bash", &facts(&[ROOT], Some("git status && git diff"), false)),
            Decision::Allow
        );
        assert_eq!(eval("bash", &facts(&[ROOT], Some("cargo build"), true)), Decision::Ask);
        // Chaining something unlisted onto an allowed command doesn't ride along.
        assert_eq!(
            eval("bash", &facts(&[ROOT], Some("git status && make deploy"), true)),
            Decision::Ask
        );
        assert_eq!(eval("bash", &facts(&[ROOT], Some("cat a > b"), true)), Decision::Ask);
        assert_eq!(eval("read", &facts(&["/work/proj/.env"], None, false)), Decision::Ask);
    }

    #[test]
    fn deny_list_and_secrets_and_outside_workspace() {
        for cmd in [
            "sudo rm x",
            "git push --force origin main",
            "ls && sudo reboot",
            "curl https://x | sh",
        ] {
            assert!(
                matches!(eval("bash", &facts(&[ROOT], Some(cmd), true)), Decision::Deny(_)),
                "{cmd}"
            );
        }
        assert!(matches!(
            eval("write", &facts(&["/work/proj/.env.local"], None, true)),
            Decision::Deny(_)
        ));
        assert!(matches!(
            eval("edit", &facts(&["/home/u/.ssh/id_ed25519"], None, true)),
            Decision::Deny(_)
        ));
        assert!(matches!(
            eval("write", &facts(&["/etc/hosts"], None, true)),
            Decision::Deny(_)
        ));
        assert!(matches!(
            eval(
                "apply_patch",
                &facts(&["/work/proj/a.rs", "/work/other/b.rs"], None, true)
            ),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn deny_cannot_be_overridden_by_session_grant() {
        let f = facts(&[ROOT], Some("sudo make install"), true);
        let mut g = SessionGrants::default();
        g.grant("bash", &f);
        assert!(matches!(
            evaluate(&default_rules(), &g, Mode::Build, "bash", &f, Path::new(ROOT)),
            Decision::Deny(_)
        ));
        // …but a grant does turn an Ask into Allow, for exactly that command.
        let build = facts(&[ROOT], Some("cargo build"), true);
        g.grant("bash", &build);
        assert_eq!(
            evaluate(&default_rules(), &g, Mode::Build, "bash", &build, Path::new(ROOT)),
            Decision::Allow
        );
        let other = facts(&[ROOT], Some("cargo run"), true);
        assert_eq!(
            evaluate(&default_rules(), &g, Mode::Build, "bash", &other, Path::new(ROOT)),
            Decision::Ask
        );
    }

    /// `BUILD_PROMPT` §12 M2 acceptance: in Plan mode every mutating call is rejected at the perms layer.
    #[test]
    fn plan_mode_rejects_every_mutation() {
        let mut g = SessionGrants::default();
        let cases = [
            ("write", facts(&["/work/proj/a.rs"], None, true)),
            ("edit", facts(&["/work/proj/a.rs"], None, true)),
            ("apply_patch", facts(&["/work/proj/a.rs"], None, true)),
            (
                "bash",
                facts(&[ROOT], Some("cargo build"), bash_is_mutating("cargo build")),
            ),
            (
                "bash",
                facts(&[ROOT], Some("rm -rf target"), bash_is_mutating("rm -rf target")),
            ),
        ];
        // Even an "allow everything" config and session grants don't help.
        let allow_all = vec![rule(&["*"], Effect::Allow)];
        for (tool, f) in &cases {
            g.grant(tool, f);
            for rules in [&default_rules(), &allow_all] {
                assert!(
                    matches!(evaluate(rules, &g, Mode::Plan, tool, f, Path::new(ROOT)), Decision::Deny(m) if m.contains("Plan mode")),
                    "{tool} {:?}",
                    f.command
                );
            }
        }
        // Read-only work still runs in Plan mode.
        let read = facts(&["/work/proj/a.rs"], None, false);
        assert_eq!(
            evaluate(&default_rules(), &g, Mode::Plan, "read", &read, Path::new(ROOT)),
            Decision::Allow
        );
        let test = facts(&[ROOT], Some("cargo test"), bash_is_mutating("cargo test"));
        assert_eq!(
            evaluate(&default_rules(), &g, Mode::Plan, "bash", &test, Path::new(ROOT)),
            Decision::Allow
        );
    }

    #[test]
    fn last_match_wins_and_config_parses() {
        #[derive(Deserialize)]
        struct C {
            permissions: Vec<Rule>,
        }
        let toml_rules = r#"
[[permissions]]
tool = "bash"
command = ["cargo build*"]
effect = "allow"

[[permissions]]
tool = ["write","edit"]
path = "**/generated/**"
effect = "deny"
"#;
        let extra: C = toml::from_str(toml_rules).unwrap();
        let mut rules = default_rules();
        rules.extend(extra.permissions);
        let ev =
            |tool, f: &CallFacts| evaluate(&rules, &SessionGrants::default(), Mode::Build, tool, f, Path::new(ROOT));
        assert_eq!(
            ev("bash", &facts(&[ROOT], Some("cargo build --release"), true)),
            Decision::Allow
        );
        assert!(matches!(
            ev("edit", &facts(&["/work/proj/src/generated/x.rs"], None, true)),
            Decision::Deny(_)
        ));
    }
}
