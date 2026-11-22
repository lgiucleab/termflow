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
- [ ] Windows: the agent's `bash` runs through PowerShell 7 and `cargo test` works.

## M2c: Unified timeline, Mono

- [ ] `cargo test` in the input box (glyph shows `$`) runs in the chat's shell and becomes a block with exit code, duration and coloured output. A question (glyph `✦`) goes to the agent. After restarting the app, both are still there, in order.
- [ ] `!make the button blue` runs as a command; `?ls` goes to the agent; Cmd/Ctrl+I locks the mode.
- [ ] `vim`, `htop` and `less` fill the timeline area and give it back cleanly when they exit.
- [ ] `read x` accepts typed input in the live block; Ctrl+C (or Stop) ends a running command.
- [ ] After a failing command, "why did that fail?" carries the block as a chip; × removes it.
- [ ] Cmd/Ctrl+J shows the classic terminal for the same shell and goes back. Running `claude` from the palette opens it there.
- [ ] A shell without integration (e.g. `sh`): after about 6 s the chat falls back to the classic terminal, and the status bar says so.
- [ ] Mono Dark is the default; Mono Light, Nord and the others switch from the palette and also recolour live blocks.
- [ ] `yes | head -n 2000000` stays responsive and freezes into a block.
- [ ] Windows (pwsh): blocks, alt-screen (`vim` via WSL or `edit`) and Ctrl+C work.

## M3a: Router with fallback

Setup: two providers (e.g. Anthropic + OpenAI) and a profile in `~/.config/termflow/termflow.toml` (see `docs/specs/M3.md` → Example). Restart the app.

- [ ] The chat header shows the profile picker; choosing the profile and sending works, and the reply says which model served it.
- [ ] Anthropic key at its spend limit (or revoke it): the next message continues on the second model at once, with a note "… unavailable (…) · continuing with …". After an app restart it still skips the blocked key.
- [ ] Mid-task switch: start a multi-step task, block the first provider while it runs (e.g. disconnect it in a proxy). The step finishes on the next model, no tool runs twice, and nothing has to be retyped.
- [ ] `budget = { session_usd = 0.01 }`: after the first reply, the next message is refused with the budget banner, and no request goes to any provider.
- [ ] Only invalid keys in the profile: the "All models in this profile are unavailable" banner lists each one.
- [ ] With no provider configured, no request goes to models.dev (check with a proxy); after adding one, the catalog loads (log: `model catalog loaded`).
- [ ] Picking a model in the header pins it (the profile picker shows "Pinned model").
