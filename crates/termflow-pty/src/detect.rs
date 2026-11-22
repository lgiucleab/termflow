//! Shell and agent-CLI discovery.

use std::path::{Path, PathBuf};

use termflow_types::{ShellKind, ShellSpec};

/// Shell family from an executable path or name.
pub fn shell_kind_for(program: &str) -> ShellKind {
    // Split on both separators so Windows paths classify correctly on any host.
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_lowercase();
    let name = file.strip_suffix(".exe").unwrap_or(&file);
    match name {
        "zsh" => ShellKind::Zsh,
        "bash" => ShellKind::Bash,
        "fish" => ShellKind::Fish,
        "nu" => ShellKind::Nu,
        "pwsh" => ShellKind::Pwsh,
        "powershell" => ShellKind::PowerShell,
        "cmd" => ShellKind::Cmd,
        "wsl" => ShellKind::Wsl,
        _ => ShellKind::Other,
    }
}

/// Find an executable on PATH (honours PATHEXT on Windows).
pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(str::to_owned)
            .chain([String::new()])
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{name}{e}")))
            .find(|p| is_executable(p))
    })
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

fn spec(id: &str, name: &str, program: &Path, args: Vec<String>) -> ShellSpec {
    let program = program.to_string_lossy().into_owned();
    ShellSpec {
        id: id.into(),
        name: name.into(),
        kind: shell_kind_for(&program),
        program,
        args,
    }
}

/// Names of every executable on PATH (deduplicated, sorted; `.exe` etc. stripped on Windows).
/// Used by the input box to tell shell commands from questions for the agent.
pub fn list_executables() -> Vec<String> {
    let Some(path) = std::env::var_os("PATH") else {
        return vec![];
    };
    let mut names = std::collections::BTreeSet::new();
    for dir in std::env::split_paths(&path) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !is_executable(&p) {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = if cfg!(windows) {
                let lower = name.to_lowercase();
                match lower.rsplit_once('.') {
                    Some((stem, "exe" | "cmd" | "bat" | "com" | "ps1")) => stem.to_owned(),
                    _ => continue,
                }
            } else {
                name
            };
            names.insert(name);
        }
    }
    names.into_iter().collect()
}

/// All shells we can find, the default one first.
pub fn detect_shells() -> Vec<ShellSpec> {
    let mut out = platform::detect();
    // De-duplicate by id, keeping the first (the default).
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.id.clone()));
    out
}

/// The user's default shell.
pub fn default_shell() -> ShellSpec {
    detect_shells().into_iter().next().unwrap_or_else(platform::fallback)
}

// Unix: $SHELL is the user's login shell; also offer the common shells if installed.
#[cfg(unix)]
mod platform {
    use super::{Path, ShellSpec, find_on_path, spec};

    pub fn detect() -> Vec<ShellSpec> {
        let mut out = Vec::new();
        if let Ok(sh) = std::env::var("SHELL") {
            let p = Path::new(&sh);
            if p.is_file() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(spec(&name, &name, p, vec![]));
            }
        }
        for name in ["zsh", "bash", "fish", "nu"] {
            if let Some(p) = find_on_path(name) {
                out.push(spec(name, name, &p, vec![]));
            }
        }
        out
    }

    pub fn fallback() -> ShellSpec {
        spec("sh", "sh", Path::new("/bin/sh"), vec![])
    }
}

// Windows: PowerShell 7 if present, else Windows PowerShell; plus cmd, Git Bash and WSL distros.
#[cfg(windows)]
mod platform {
    use super::{Path, PathBuf, ShellSpec, find_on_path, spec};

    pub fn detect() -> Vec<ShellSpec> {
        let mut out = Vec::new();
        if let Some(p) = find_on_path("pwsh") {
            out.push(spec("pwsh", "PowerShell 7", &p, vec![]));
        }
        if let Some(p) = find_on_path("powershell") {
            out.push(spec("powershell", "Windows PowerShell", &p, vec![]));
        }
        let cmd = std::env::var("COMSPEC")
            .map(PathBuf::from)
            .ok()
            .or_else(|| find_on_path("cmd"));
        if let Some(p) = cmd {
            out.push(spec("cmd", "Command Prompt", &p, vec![]));
        }
        for base in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Ok(b) = std::env::var(base) {
                for rel in [r"Git\bin\bash.exe", r"Programs\Git\bin\bash.exe"] {
                    let p = Path::new(&b).join(rel);
                    if p.is_file() {
                        out.push(spec("git-bash", "Git Bash", &p, vec!["-l".into()]));
                    }
                }
            }
        }
        out.extend(wsl_distros());
        out
    }

    fn wsl_distros() -> Vec<ShellSpec> {
        let Some(wsl) = find_on_path("wsl") else { return vec![] };
        let Ok(o) = std::process::Command::new(&wsl).args(["-l", "-q"]).output() else {
            return vec![];
        };
        // wsl.exe prints UTF-16LE.
        let units: Vec<u16> = o
            .stdout
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .lines()
            .map(|l| l.trim().trim_matches('\0'))
            .filter(|l| !l.is_empty())
            .map(|d| {
                spec(
                    &format!("wsl:{d}"),
                    &format!("WSL: {d}"),
                    &wsl,
                    vec!["-d".into(), d.into()],
                )
            })
            .collect()
    }

    pub fn fallback() -> ShellSpec {
        spec(
            "cmd",
            "Command Prompt",
            Path::new(r"C:\Windows\System32\cmd.exe"),
            vec![],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(shell_kind_for("/bin/zsh"), ShellKind::Zsh);
        assert_eq!(
            shell_kind_for(r"C:\Program Files\PowerShell\7\pwsh.exe"),
            ShellKind::Pwsh
        );
        assert_eq!(shell_kind_for("powershell.exe"), ShellKind::PowerShell);
        assert_eq!(shell_kind_for("/opt/x/thing"), ShellKind::Other);
    }

    #[test]
    fn lists_executables() {
        let all = list_executables();
        assert!(
            all.iter().any(|n| n == "sh" || n == "cmd" || n == "pwsh"),
            "{}",
            all.len()
        );
    }

    #[test]
    fn detects_at_least_one_shell() {
        let shells = detect_shells();
        assert_ne!(shells.len(), 0);
        let ids: std::collections::HashSet<_> = shells.iter().map(|s| &s.id).collect();
        assert_eq!(ids.len(), shells.len());
    }
}
