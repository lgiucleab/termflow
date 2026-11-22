# Manual test checklist

Things CI or the dev machine (macOS arm64) cannot verify. Tick per release.

## M0
- [ ] **macOS x64** (Intel Mac or Rosetta): open the `termflow-macos-x64` `.dmg` from CI, drag to Applications, right-click → Open. Window shows "Version 0.1.0" and the event counter increases every second.
- [ ] **Windows x64**: install the `.msi` and separately the NSIS `.exe` from CI (SmartScreen: More info → Run anyway). Same checks as above.
- [ ] Windows: logs are written under `%LOCALAPPDATA%\flowlabs\termflow\data\logs`.

## M1 — Terminal

Run on **macOS** and **Windows (PowerShell 7 and Git Bash)**.

- [ ] `vim`, `less`, `git log`, `htop`/`btop` render with no artifacts; quitting restores the prompt.
- [ ] `claude` and `codex` (if installed) via palette → "New external agent": TUI renders, no flicker while streaming a long answer.
- [ ] Resize the window and drag split dividers while output is streaming (`yes`, or an agent answering): no corruption.
- [ ] Each command shows a block header with the right exit badge, duration, cwd and branch (zsh, bash, fish, pwsh).
- [ ] `ssh` into a host without integration: the session stays usable; the `ssh` command is one block.
- [ ] Windows: if blocks show "unavailable" in pwsh, note the Windows build (ConPTY reordering).
- [ ] Windows: Ctrl+C copies with a selection and interrupts without one; Ctrl+V pastes; Ctrl+Shift+T/W/D/E/K/F work.
- [ ] Multi-line paste asks first; single-line paste doesn't.
- [ ] Clicking an http(s) link opens the default browser.
- [ ] Quit and reopen: tabs and splits come back (as plain shells).
- [ ] Spanish UI (system language es-*): all chrome strings are translated.
