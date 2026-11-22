# Shell integration

Scripts embedded into the TermFlow binary (`termflow-shell-integ`) and written to the app data dir at startup.
They never edit the user's rc files. Marks emitted (BEL-terminated OSC):

| Sequence | Meaning |
|---|---|
| `OSC 133;A` | prompt start |
| `OSC 133;B` | prompt end / command input start |
| `OSC 6973;cmd=<base64>` | the command line about to run |
| `OSC 133;C` | command executed / output start |
| `OSC 133;D;<exit>` | command finished |
| `OSC 6973;cwd=<path>` | working directory |
| `OSC 6973;git_branch=<name>` | current branch (empty = none) |

| Shell | Injection |
|---|---|
| zsh | `ZDOTDIR` points at `zsh/`; its `.zshenv` restores the user's `ZDOTDIR`, sources their `.zshenv`, loads `integration.zsh`. |
| bash | `--rcfile bash/rc.bash`, which sources the user's login files (macOS, Git Bash) or `~/.bashrc`, then `integration.bash`. bash 3.2 compatible. |
| fish | `--init-command 'source $TERMFLOW_SHELL_INTEGRATION'` (runs after `config.fish`). |
| pwsh / powershell | `-NoExit -Command` evaluates `integration.ps1` as a script block (works under any execution policy). |

Opt out with `terminal.shell_integration = false`.
