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

## M2a: Models and chat

For a no-key smoke test, run `cargo run -p mock-provider -- tests/fixtures/http/ollama/text_tool.ndjson`, add a provider **Ollama** with that URL as base URL, use model `local-test-model`, and send a message.

- [ ] Providers → add **Anthropic** with your API key. "Test" shows the model count, and the key field is empty again after adding.
- [ ] Repeat for **OpenAI**, **OpenRouter** and **Ollama** (install Ollama and run `ollama pull <a coder model>` first).
- [ ] An Agent tab (palette → New agent session): pick provider + model → Use → send a message. Text streams; reasoning shows collapsed; "served by" names the model.
- [ ] Send a second message in the same session: no error. This proves that thinking/reasoning blocks are replayed correctly to Anthropic and OpenAI.
- [ ] Esc during a long answer keeps the partial text with "[cancelled]".
- [ ] Quit the app mid-answer, then reopen: the session shows up to the last finished answer and continues normally.
- [ ] A wrong API key shows "The API key was rejected." A non-reasoning model with Reasoning on shows the provider's error; turning Reasoning off fixes it.
- [ ] macOS Keychain Access shows items under `com.flowlabs.termflow`; removing a provider in the app removes its item.

## M2b: Tools, agent loop, chat-first layout

- [ ] First start: the start screen shows "Open folder…"; picking a folder opens it with a new chat; reopening the app returns to it.
- [ ] Sidebar: new chat, rename (double-click), delete (confirm). The working dot shows while the agent runs.
- [ ] In a real Rust project with a real key: "find the failing test and fix it". The agent reads/greps without asking, shows an approval card with a diff for the edit, runs `cargo test` without asking, and reports.
- [ ] Deny with feedback: the agent adapts instead of retrying the same call.
- [ ] Shift+Tab to Plan: the agent explores and writes a todo plan; edits are refused with "Plan mode".
- [ ] Esc while a long `bash` runs (e.g. `sleep 60`): the command is killed and the card says cancelled.
- [ ] `/help`, `/cost`, `/init` (creates AGENTS.md after approval), `/clear` (new chat), `/attach` (attaches the last terminal block).
- [ ] `@` picker lists project files (not `.env`); the attached file shows collapsed in the message.
- [ ] Cmd/Ctrl+J toggles the terminal panel; terminals open in the project folder; drag to resize.
- [ ] Windows: the agent's `bash` runs through PowerShell 7 and `cargo test` works.
