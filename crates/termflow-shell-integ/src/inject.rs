//! Writes the embedded integration scripts to disk and builds the args/env that load them.

use std::path::Path;

use termflow_types::ShellKind;

const FILES: &[(&str, &str)] = &[
    ("zsh/.zshenv", include_str!("../../../shell-integration/zsh/.zshenv")),
    (
        "zsh/integration.zsh",
        include_str!("../../../shell-integration/zsh/integration.zsh"),
    ),
    ("bash/rc.bash", include_str!("../../../shell-integration/bash/rc.bash")),
    (
        "bash/integration.bash",
        include_str!("../../../shell-integration/bash/integration.bash"),
    ),
    (
        "fish/integration.fish",
        include_str!("../../../shell-integration/fish/integration.fish"),
    ),
    (
        "pwsh/integration.ps1",
        include_str!("../../../shell-integration/pwsh/integration.ps1"),
    ),
];

/// Write (overwrite) all scripts under `dir`. Call once at startup.
pub fn write_scripts(dir: &Path) -> std::io::Result<()> {
    for (rel, body) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
    }
    Ok(())
}

/// Extra arguments (placed before the shell's own) and environment for one shell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Injection {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// How to load integration into `kind`, or `None` if we don't support it (cmd, nu, wsl, other).
/// `user_env` reads the environment the shell would otherwise inherit.
pub fn inject(kind: ShellKind, dir: &Path, user_env: impl Fn(&str) -> Option<String>) -> Option<Injection> {
    let mut inj = Injection::default();
    match kind {
        ShellKind::Zsh => {
            if let Some(z) = user_env("ZDOTDIR") {
                inj.env.push(("TERMFLOW_USER_ZDOTDIR".into(), z));
            }
            inj.env.push(("ZDOTDIR".into(), path(&dir.join("zsh"))));
            inj.env.push((
                "TERMFLOW_SHELL_INTEGRATION".into(),
                path(&dir.join("zsh/integration.zsh")),
            ));
            inj.args.push("-l".into());
        }
        ShellKind::Bash => {
            // macOS and Git Bash users expect login-shell startup files; Linux uses ~/.bashrc.
            let login = if cfg!(any(target_os = "macos", windows)) {
                "1"
            } else {
                "0"
            };
            inj.env.push(("TERMFLOW_BASH_LOGIN".into(), login.into()));
            inj.env.push((
                "TERMFLOW_SHELL_INTEGRATION".into(),
                path(&dir.join("bash/integration.bash")),
            ));
            inj.args.extend(["--rcfile".into(), path(&dir.join("bash/rc.bash"))]);
        }
        ShellKind::Fish => {
            inj.env.push((
                "TERMFLOW_SHELL_INTEGRATION".into(),
                path(&dir.join("fish/integration.fish")),
            ));
            inj.args
                .extend(["--init-command".into(), "source $TERMFLOW_SHELL_INTEGRATION".into()]);
        }
        ShellKind::Pwsh | ShellKind::PowerShell => {
            inj.env.push((
                "TERMFLOW_SHELL_INTEGRATION".into(),
                path(&dir.join("pwsh/integration.ps1")),
            ));
            inj.args.extend([
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                // A script block (not a .ps1 file) is not subject to the execution policy.
                ". ([scriptblock]::Create((Get-Content -Raw -LiteralPath $env:TERMFLOW_SHELL_INTEGRATION)))".into(),
            ]);
        }
        ShellKind::Nu | ShellKind::Cmd | ShellKind::Wsl | ShellKind::Other => return None,
    }
    Some(inj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_every_script() {
        let d = tempfile::tempdir().unwrap();
        write_scripts(d.path()).unwrap();
        for (rel, _) in FILES {
            assert!(d.path().join(rel).is_file(), "{rel}");
        }
    }

    #[test]
    fn zsh_preserves_user_zdotdir() {
        let d = Path::new("/x");
        let inj = inject(ShellKind::Zsh, d, |k| {
            (k == "ZDOTDIR").then(|| "/home/u/.config/zsh".into())
        })
        .unwrap();
        assert!(
            inj.env
                .contains(&("TERMFLOW_USER_ZDOTDIR".into(), "/home/u/.config/zsh".into()))
        );
        assert!(inj.env.iter().any(|(k, v)| k == "ZDOTDIR" && v.ends_with("zsh")));
        let none = inject(ShellKind::Zsh, d, |_| None).unwrap();
        assert!(!none.env.iter().any(|(k, _)| k == "TERMFLOW_USER_ZDOTDIR"));
    }

    #[test]
    fn unsupported_shells_get_nothing() {
        for k in [ShellKind::Cmd, ShellKind::Nu, ShellKind::Wsl, ShellKind::Other] {
            assert!(inject(k, Path::new("/x"), |_| None).is_none());
        }
    }
}
