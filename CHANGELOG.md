# Changelog

All notable changes to this project. Format: Keep a Changelog; versions follow SemVer.

## [Unreleased]

### Changed (M2d — OpenCode-style UI)
- **New look**, modelled on OpenCode's GUI (MIT), still Mono by default:
  - floating raised panels on a darker window layer
  - Inter for the UI and IBM Plex Mono for code
  - a 36px title bar with the project and a command search
  - two-line session rows (mode · model · time) in a resizable sidebar (⌘⇧B / Ctrl+Shift+B)
- **Timeline:**
  - tool calls are one-line rows that expand on click
  - back-to-back reads, greps and globs collapse into "Explored N files"
  - thinking is a collapsed row
  - user messages are right-aligned bubbles
  - errors and route switches are notice rows
- **Composer:**
  - a raised card that grows up to 180px
  - Build/Plan agent switch (a filled or hollow dot), model chip with a picker popover, command/agent glyph, and a send/stop/run button
  - a todo pill when the agent has a plan
- **Permission requests** dock above the composer (Deny with feedback / Deny / Allow for session / Allow once) instead of sitting inside the tool card.
- **Right panel** (⌘⇧R / Ctrl+Shift+R) with the agent's Todo list and the Context attached to your next message.
- **The classic terminal** (⌘J) is now a resizable bottom panel. Without shell integration it fills the session.
- **Settings** is a full-screen page (⌘,):
  - Appearance: theme cards and timeline detail (Everything / Compact / Messages only)
  - Providers (moved out of the dialog)
  - Shortcuts
  - About
- **Command palette:** grouped results; ⌘⇧P also opens it.
- The status bar is gone: shell state shows in the session header, and Providers and the palette are in the title bar.

### Added (M3a — Router with fallback)
- **Fallback profiles** (`[profiles.<name>]`): a chat can follow an ordered chain of models across providers or your own distinct keys. Rate limits wait or move on, quota exhaustion moves on at once (blocked until the reset, remembered across restarts), outages retry with backoff then move on, oversized requests go to `on_context_overflow` or have old tool output trimmed, and refusals go to `on_content_policy`.
- A failure mid-answer continues on the next model without retyping; tools that already ran are never run again.
- Proactive switching from rate-limit headers, hysteresis before switching back, and an automatic pause/resume when every model is down.
- Budgets per session/day/month with a hard stop that never silently switches to another paid model; every attempt is priced from the model that actually served it.
- Model catalog from models.dev (cached, refreshed at most daily, only once a provider is configured) with `[catalog.overrides]`.

### Changed (M2c — Unified timeline, Mono)
- **One timeline per chat** (decision 0002): shell commands and agent turns share one stream and one input box. Commands run in the chat's own shell and appear as Warp-style blocks (command, exit, duration, coloured output, copy/ask/re-run). Full-screen programs take over the timeline, and Cmd/Ctrl+J shows the classic terminal view.
- The input auto-detects command vs question (`$` / `✦` glyph); `!`, `?` and Cmd/Ctrl+I override it. Commands run since the agent's last reply are attached to your next question as removable chips.
- **Mono Dark** (new default) and **Mono Light** themes: black and white with green/red only for status. WCAG AA is checked in tests.

### Removed (M2c)
- Terminal tabs, splits and the bottom terminal panel (replaced by the timeline and the classic terminal view).

### Added (M2b — Tools, agent loop, chat-first)
- **Chat-first layout** (decision 0001): start screen with recent projects, sidebar of chats per project, terminal as a toggleable panel (Cmd/Ctrl+J) that opens in the project folder.
- The agent works in the project folder with 11 tools: read, write, edit, apply_patch (V4A), bash, grep, glob, ls, todo, attach_block, webfetch.
- Permission rules (Balanced defaults, deny list, `[[permissions]]` in config) with approval cards that show the exact diff or command; Plan mode is read-only, enforced in the permission layer.
- Project instructions (AGENTS.md/CLAUDE.md/.termflow/rules), environment summary, `@file` and `@block` mentions, slash commands (`/plan /build /model /clear /attach /init /cost /help`).
- A tool call never runs twice; calls interrupted by a crash are reported, not re-run.

### Added (M2a — Models and chat)
- Provider connections: Anthropic, OpenAI, OpenRouter, Ollama, or any custom endpoint (OpenAI Chat / Responses / Anthropic format). API keys live only in the OS keychain.
- Canonical transcript (append-only, provider-native reasoning kept verbatim) with crash recovery to the last finished turn.
- Four streaming adapters with error classification (rate limit vs quota vs overload vs context overflow vs auth…) and model listing.
- Router pass-through with a per-request attempt log (tokens, cache, TTFT, served model, error class).
- Agent tab: streaming chat with model picker, reasoning toggle, safe Markdown, Stop/Esc, and attached terminal blocks as context.
- Mock provider (tests and dev server) and fixtures for all four wire formats.

### Added (M1 — Terminal)
- PTY sessions (`portable-pty`, ConPTY on Windows) with shell detection (zsh, bash, fish, nu; pwsh, Windows PowerShell, cmd, Git Bash, WSL distros) and agent-CLI discovery on PATH.
- Shell integration for zsh, bash (3.2+), fish and PowerShell, injected without touching rc files; OSC 133 / 6973 / 7 parsed into blocks (command, exit code, duration, cwd, branch), stored in SQLite with a 2 MB plain-text output excerpt.
- Degradation: out-of-order marks switch blocks off for that pane with an indicator.
- Output batching (8 ms / 64 KiB) over a raw-bytes Tauri channel, with back-pressure.
- Terminal UI: xterm.js 6 + WebGL (DOM fallback), tabs, splits with drag-to-resize, rename, layout restore, find, links, terminal-aware copy/paste, multi-line paste warning.
- Inline block headers with copy command/output/markdown, attach to agent, re-run, collapse.
- Placeholder agent tab listing attached blocks; command palette; 6 themes (Dark, Light, High Contrast, Nord, Gruvbox Dark, Catppuccin Mocha); bundled JetBrains Mono (OFL).
- Dev-only perf monitor and `TERMFLOW_DEV_AUTORUN` harness; CI runs real-shell tests on macOS and Windows.

### Removed
- M0 demo ticker.

### Added (M0 — Skeleton)
- Cargo workspace with all planned crates (v1/v2 crates are documented stubs).
- `termflow-types`: ULID ids, `AppInfo`, `AppEvent`, `ErrorClass`.
- `termflow-store`: SQLite (WAL, foreign keys), single writer thread, migration `0001_init`.
- `termflow-config`: global + project TOML merge, unknown keys rejected, JSON schema in `schemas/`.
- `termflow-secrets`: redaction of known secrets and common token patterns.
- `termflow-core`: composition root, event bus, rolling redacted logs.
- Desktop app (Tauri 2 + SolidJS + Tailwind v4) with typed bridge (tauri-specta), en / es-ES strings, version display and live core-event counter.
- CI: checks on Linux; unsigned installers for macOS arm64, macOS x64, Windows x64.
