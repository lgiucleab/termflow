# Build Prompt for Claude Code — "TermFlow": a Warp/OpenCode-style desktop app for AI coding agents

> **How to use this file:** Put this file in the root of an empty Git repository as `docs/BUILD_PROMPT.md`, open Claude Code in that folder and say:
> *"Read docs/BUILD_PROMPT.md in full. Follow Section 0 (Operating Instructions) exactly. Start with Milestone M0."*
>
> **Product name: TermFlow** (by Flow Labs, alongside TrainFlow, FileFlow and FinanceFlow). Naming conventions: crates `termflow-*`, CLI command `termflow`, config files `termflow.toml` / `.termflow/`, ignore file `.termflowignore`, env vars `TERMFLOW_*`, bundle identifier `com.flowlabs.termflow`, suggested GitHub repo `flowlabs/termflow`. The bare name `termflow` is already used by small unrelated projects on some package registries (a Scala TUI library, a Haskell library), so do not try to publish packages under the bare name without checking first.

---

## 0. Operating instructions for Claude Code (read first, obey always)

You are the lead engineer building a production-quality, cross-platform desktop application. The person who gave you this file is the product owner. They direct the work and review it; you write the code.

1. **Read this whole document before writing any code.** It is the source of truth. If something here conflicts with your defaults or habits, this document wins. If something here is technically wrong or out of date (e.g., a crate API has changed), **stop and tell me**, propose a fix, and wait for approval. Do not silently deviate.
2. **Work milestone by milestone** (Section 12). For each milestone:
   1. Create `docs/specs/<milestone>.md` expanding that milestone's scope into concrete tasks, files, interfaces and tests.
   2. Show me the plan and **wait for my approval** before implementing.
   3. Write the acceptance tests first (TDD), wherever that's practical.
   4. Implement in small commits on a branch named `m<N>-<short-name>`.
   5. Run the full check suite (Section 14.3) and fix everything until it's green.
   6. Update `docs/specs/<milestone>.md` with what was actually built, deviations, and known issues. Update `CHANGELOG.md`.
   7. Give me a short summary: what works, how to try it, what's left, risks.
3. **Verify library versions and APIs before pinning them.** Several crates in this spec were chosen from research but not hands-on verified (marked **[verify]**). Before adding each one, check the latest stable version and its current API (docs.rs, crates.io, npm, GitHub README). If a crate is unmaintained or unsuitable, propose an alternative.
4. **Never invent model IDs.** Model names change constantly. Use the model catalog (Section 7.9) or placeholders like `<anthropic-flagship>` in examples and defaults. Hard-coded model IDs may only appear in test fixtures.
5. **Ask before:** adding a new top-level dependency not listed here, changing the stack, changing the architecture rules in Section 3, deleting user data, or touching anything listed under the legal constraints in Section 9.
6. **Don't ask about:** naming of internal functions, file layout inside a crate, test structure, small refactors. Make the call and mention it in the summary.
7. Keep `CLAUDE.md` and `AGENTS.md` (Section 15) up to date as the project evolves. They are your memory between sessions.
8. Platform-specific code goes behind `#[cfg(target_os = "...")]` in dedicated modules (`unix.rs`, `windows.rs`, `macos.rs`), with a comment saying why it's platform-specific.
9. When you can't test something on the current machine (e.g., Windows behaviour from a Mac), say so explicitly in the summary and add it to a `docs/MANUAL_TEST_CHECKLIST.md` for me to run on that platform.

---

## 1. Product vision

### 1.1 One sentence
A fast, native-feeling desktop app for macOS and Windows that combines **a modern block-based terminal (like Warp)**, **an open, provider-agnostic coding agent (like OpenCode)**, and **a local model router with automatic fallback (like OpenRouter, but in-app)**. When one model or provider runs out of quota, gets rate-limited or goes down, the same session carries on with the next model, without losing context and without the user retyping anything.

### 1.2 Why it exists (the gaps we're filling)
- **Quota and rate-limit pain** is the #1 complaint about AI coding tools in 2026. Users hit limits mid-task and lose flow.
- **OpenCode has no native cross-model fallback.** It only falls back across providers when the model ID is identical (opencode issue #7602).
- **Warp is cloud- and credit-centric**, and has drawn criticism for its login requirement and telemetry. **OpenCode is heavy** (reports of ~1 GB RAM for a TUI) and once sent prompts to a cloud model the user hadn't configured.
- **Third-party agent CLIs (Claude Code, Codex) flicker and render badly in Windows terminals.** A terminal that hosts them well is valuable on its own.
- **No tool hands off between agents from different vendors.** For example: "Claude Code hit its limit → continue this task in Codex CLI or our own agent on the same worktree, with a hand-off summary."

### 1.3 Target user
Individual developers and students who use several AI providers (API keys, local models, official CLIs). They care about speed, privacy, cost control and not being blocked by a single vendor's quota.

### 1.4 Core principles (non-negotiable)
1. **Speed:** cold start < 1 s, idle RAM < 250 MB with one session and one terminal open, no dropped frames while streaming > 100 tokens/s. These are targets; measure them in CI where possible (Section 14.4).
2. **Local-first and private:** zero telemetry by default. **No network request leaves the machine unless it goes to a provider the user explicitly configured.** Auxiliary tasks (session titles, compaction) use only models the user configured, never an implicit cloud model.
3. **Never lose a session:** the canonical transcript is append-only, persisted on every event, and survives crashes and model switches.
4. **The user is in control:** every destructive or external action goes through permission rules. Everything is undoable via snapshots.
5. **Provider-agnostic:** no provider gets special treatment in the core. Providers are adapters.
6. **Legally clean:** see Section 9. We never abuse consumer subscription tokens.

---

## 2. Tech stack (decided — do not change without approval)

| Layer | Choice | Notes |
|---|---|---|
| Language (core) | **Rust stable** (latest), edition 2024 if supported by all deps, else 2021 | Cargo workspace |
| Async runtime | **tokio** (multi-threaded) | |
| Desktop shell | **Tauri 2** | WebView2 on Windows, WKWebView on macOS. Core runs **in-process** (no sidecar binary for the core). |
| Frontend | **SolidJS + TypeScript (strict) + Vite + Tailwind CSS v4** | Chosen for fine-grained reactivity under high-frequency token streams |
| Terminal rendering | **xterm.js** + `@xterm/addon-webgl`, `addon-fit`, `addon-search`, `addon-unicode11`, `addon-web-links`, `addon-serialize` **[verify package names]** | Wrapped behind a `TerminalView` interface so it can be swapped for `ghostty-web` later |
| PTY | **`portable-pty`** (ConPTY on Windows) **[verify]** | |
| VT parsing in Rust | **`alacritty_terminal`** (headless mode, block detection fallback, future native UI) | xterm.js parses VT for display; Rust parses OSC 133 for blocks |
| HTTP / streaming | **`reqwest`** (rustls) + **`eventsource-stream`** or equivalent SSE parser **[verify]**, `serde`, `serde_json` | |
| Persistence | **SQLite** via **`rusqlite`** (bundled) + versioned migrations (`rusqlite_migration` or `refinery`) **[verify]** | WAL mode |
| Secrets | **`keyring`** crate (macOS Keychain / Windows Credential Manager) **[verify]** | Secrets never written to disk or logs |
| MCP | **`rmcp`** (official Rust MCP SDK, 3.x; stdio + Streamable HTTP) | v1 |
| Search | **`ignore`** + **`grep`** crates (ripgrep internals), **`globset`** | |
| Code parsing | **`tree-sitter`** + grammars (rust, ts/js, python, go, java, c/c++, json, toml, yaml, markdown) | Used for outline/symbol tools in v1; semantic index in v2 |
| Diff / patch | **`similar`** (diffs) + our own apply-patch implementation (Section 6.3) | |
| Snapshots | **`gix`** (gitoxide) for an internal shadow repo **[verify]**; fallback: shell out to `git` | |
| Embeddings / vectors (v2) | **`sqlite-vec`** | |
| Logging | **`tracing`** + `tracing-subscriber` + rolling file appender, with a redaction layer | |
| Errors | `thiserror` in libraries, `anyhow` only in binaries / Tauri glue | |
| Config | **TOML** (`toml`, `serde`), JSON Schema generated for editor autocomplete | |
| Updates | **`tauri-plugin-updater`** **[verify]** | Signed update manifests |
| Frontend libs | `marked` or `markdown-it` + `shiki` (code highlighting), `@tanstack/solid-virtual` (virtualized lists) **[verify]**, a diff viewer (build our own on top of `diff` or use a maintained Solid/vanilla one) | Keep the bundle small |
| Package managers | `cargo`, `pnpm` | |
| CI | **GitHub Actions**: macOS arm64, macOS x64, Windows x64 (Linux x64 build for tests only) | |

### 2.1 Escape hatches (architecture must keep them open)
- **If WKWebView causes serious rendering problems on macOS** (OpenCode moved from Tauri to Electron for this reason): we can swap the shell for Electron and reuse the same SolidJS UI. So **no business logic lives in `src-tauri`**, and the UI talks to the core only through one typed bridge module (`ui/src/bridge/`).
- **If raw performance becomes the product:** a native GPUI UI (Zed's framework, plus the `gpui-component` kit) could be added later as another client of the same core. So the core crates must not depend on Tauri.

---

## 3. Architecture rules (non-negotiable)

1. **The canonical transcript (`termflow-types`) is the single source of truth.** Never mutate or rebuild past turns. Provider-native blocks (thinking, signatures, encrypted reasoning) are stored verbatim alongside the neutral representation.
2. **Every model call goes through `termflow-router`.** The agent, the UI and the tools never call a provider directly.
3. **Providers classify errors; the router decides.** Each adapter converts raw HTTP and SSE errors into a typed `ErrorClass` (Section 7.4). Only the router decides whether to retry, cool down or switch.
4. **Shell integration uses OSC sequences only (OSC 133 + our own OSC 6973), never DCS.** ConPTY drops custom DCS and can reorder OSC, and Warp hit exactly this.
5. **`src-tauri` is thin glue.** Tauri commands deserialize input, call a core service, and serialize output. No logic.
6. **Zero telemetry.** No analytics SDKs. Crash reports are written locally; the user can choose to share them manually.
7. **Secrets only live in the OS keychain** (`termflow-secrets`). Config files reference them as `secret://<id>`.
8. **Every tool call goes through `termflow-perms`** before execution, and every file mutation goes through `termflow-snapshots` first.
9. **All core services are async, cancellable** (`tokio_util::sync::CancellationToken`) and emit events on a typed event bus; the UI subscribes to it.
10. **No `unwrap()`/`expect()` in non-test library code** except for provably-infallible cases with a comment.

---

## 4. Repository layout

```
termflow/
├── Cargo.toml                    # workspace, shared deps, lints
├── rust-toolchain.toml
├── CLAUDE.md                     # instructions for you (Section 15)
├── AGENTS.md                     # same essentials, open standard
├── CHANGELOG.md
├── LICENSE                       # ask the owner (suggest MIT or Apache-2.0)
├── crates/
│   ├── termflow-types/             # canonical transcript, events, ids, ErrorClass, shared DTOs (serde + ts-rs)
│   ├── termflow-config/            # TOML config load/merge (global → project), JSON schema, hot reload
│   ├── termflow-store/             # rusqlite, migrations, repositories (sessions, turns, attempts, costs, blocks)
│   ├── termflow-secrets/           # keyring wrapper, secret:// resolution, redaction registry
│   ├── termflow-pty/               # PTY spawn/resize/read/write, shell detection, ConPTY quirks
│   ├── termflow-shell-integ/       # OSC 133/6973 parser → Block events; injected shell scripts
│   ├── termflow-terminal/          # terminal session manager: PTYs + blocks + scrollback + agent detection
│   ├── termflow-catalog/           # model catalog (models.dev / OpenRouter models API), capabilities, prices
│   ├── termflow-providers/         # adapters: anthropic, openai_responses, openai_chat, gemini, openrouter,
│   │                             #   vercel_gateway, openai_compatible (generic), ollama, lmstudio
│   ├── termflow-router/            # classifier glue, health/circuit breaker, budget ledger, context fitter,
│   │                             #   capability filter, fallback chains, hysteresis, attempt log
│   ├── termflow-tools/             # built-in tools (read, write, edit, apply_patch, bash, grep, glob, ls,
│   │                             #   webfetch, todo, attach_block, outline)
│   ├── termflow-perms/             # ordered allow/ask/deny rules, approval broker
│   ├── termflow-snapshots/         # shadow git repo per workspace, undo/redo
│   ├── termflow-agent/             # agent loop, modes (build/plan), compaction, system prompts, subagents
│   ├── termflow-context/           # AGENTS.md/CLAUDE.md discovery, skills loader, @-mentions, ignore rules
│   ├── termflow-mcp/               # MCP client via rmcp (v1)
│   ├── termflow-acp/               # Agent Client Protocol client + registry (v1)
│   ├── termflow-sandbox/           # seatbelt (macOS), bwrap/seccomp (Linux), restricted user (Windows) (v1)
│   ├── termflow-worktrees/         # git worktree lifecycle for parallel sessions (v1)
│   ├── termflow-core/              # composition root: wires services, event bus, app state
│   └── termflow-daemon/            # local HTTP+WebSocket API for CLI/other clients (v1)
├── apps/
│   ├── desktop/
│   │   ├── src-tauri/            # Tauri 2 app: commands + event forwarding only
│   │   └── ui/                   # SolidJS app
│   │       ├── src/bridge/       # the ONLY module that calls Tauri invoke/listen
│   │       ├── src/stores/       # Solid stores mirroring core state
│   │       ├── src/components/
│   │       ├── src/features/     # terminal/, agent/, diff/, models/, settings/, palette/, agents-panel/
│   │       └── src/styles/
│   └── cli/                      # `termflow` headless CLI (v1)
├── shell-integration/            # zsh, bash, fish, pwsh scripts (embedded into binary at build time)
├── tests/
│   ├── fixtures/http/            # recorded provider responses (JSON + SSE transcripts)
│   ├── mock-provider/            # tiny axum server replaying fixtures & scripted failures
│   └── e2e/                      # WebDriver/Playwright-style end-to-end tests
├── docs/
│   ├── BUILD_PROMPT.md           # this file
│   ├── ARCHITECTURE.md           # you write and maintain this
│   ├── MANUAL_TEST_CHECKLIST.md
│   └── specs/                    # one spec per milestone
└── .github/workflows/            # ci.yml, release.yml
```

Generate TypeScript types from Rust DTOs with **`ts-rs`** or **`specta`** **[verify; prefer `specta` + `tauri-specta` if it supports Tauri 2 well]**. Then the bridge is fully typed and never written by hand twice.

---

## 5. Terminal subsystem

### 5.1 PTY (`termflow-pty`)
- Spawn the user's login shell by default: macOS `$SHELL` (usually zsh); Windows PowerShell 7 (`pwsh`) if present, else Windows PowerShell 5.1. The user can pick from detected shells: zsh, bash, fish, nu, pwsh, powershell, cmd, Git Bash, and WSL distros (`wsl.exe -l -q`).
- Environment: inherit, then set `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=TermFlow`, `TERM_PROGRAM_VERSION`, `TERMFLOW_SESSION_ID`.
- Read the PTY on a dedicated blocking thread and forward bytes over a bounded channel. Batch output to the UI at most every ~8 ms (one frame at 120 Hz) or 64 KB, whichever comes first.
- Resize propagation with debounce. Handle the child process exiting, and support kill and Ctrl+C (signal on Unix, ConPTY on Windows).
- **Windows ConPTY quirks:** use the newest ConPTY available. Document whether to ship a bundled `conpty.dll`/`OpenConsole.exe` like Windows Terminal/VS Code do **[verify feasibility and licensing; ask before shipping]**. Expect OSC reordering and add a degradation mode (5.2).

### 5.2 Shell integration and blocks (`termflow-shell-integ`)
- Inject integration scripts automatically (without editing the user's rc files): zsh via `ZDOTDIR` shim; bash via `--rcfile` shim that sources the user's `.bashrc`; fish via `XDG_DATA_DIRS`/`--init-command`; PowerShell via `-NoExit -Command ". 'script.ps1'"`. Provide an opt-out setting.
- Emit **OSC 133** marks: `A` (prompt start), `B` (prompt end / command input start), `C` (command executed / output start), `D;<exit>` (command finished).
- Emit our own **OSC 6973** for metadata: `cwd=<path>`, `git_branch=<name>`, `cmd=<base64 command line>`, `duration_ms`, `shell=<name>`. Also accept the standard OSC 7 for cwd.
- Turn the marks into **Blocks**: `{block_id, session_id, command, cwd, git_branch, started_at, finished_at, exit_code, output_range (scrollback line range), shell}`. Persist the metadata in SQLite, and keep the output in the scrollback, with an optional capped copy of plain text (e.g., the last 2 MB per block, ANSI stripped) for agent context.
- **Degradation mode:** if marks arrive out of order or are missing (ConPTY, `ssh` into a host without integration), fall back to heuristic prompt detection with `alacritty_terminal`'s grid, or turn blocks off for that pane. Show a small indicator saying blocks are unavailable.
- Alternate screen (vim, htop, less, Claude Code's TUI): while it's active, block grouping is suspended for that pane.

### 5.3 Terminal UI
- xterm.js with the WebGL renderer (fall back to canvas/DOM if WebGL isn't available), font ligatures optional, true color, unicode11, clickable links (open in the OS browser via Tauri shell plugin, with confirmation for non-http schemes), search (Cmd/Ctrl+F), copy on select optional, bracketed paste, a warning when pasting multiple lines.
- **Block UX** (above the terminal or as an overlay gutter): each finished command shows a header bar with the command, exit code badge (green/red), duration, cwd and branch. Block actions: copy command, copy output, **"Attach to agent"**, re-run, collapse/expand, share as markdown (local copy only).
- **Input editor (MVP-lite, v1-full):** in MVP use xterm's native input. In v1, add an optional IDE-style input editor above the prompt (multi-line, mouse selection, Cmd/Ctrl+Z, syntax highlighting of shell, history search). It's only active when shell integration reports we're at a prompt (after `OSC 133;B`), and it sends the composed command to the PTY on Enter.
- Tabs, split panes (horizontal/vertical, drag to resize), pane focus navigation, rename tab, per-tab shell profile, restore the tab layout on restart (v1: also restore cwd).
- **Command palette** (Cmd/Ctrl+K): fuzzy search over actions, open sessions, recent blocks, workflows (v1), settings, model profiles.
- Themes: ship 6+ themes (dark default, light, high-contrast and some popular palettes), import of iTerm2/Windows Terminal JSON themes (v1). The UI theme and terminal theme are linked but separately overridable.

### 5.4 Hosting external agent CLIs (MVP)
The legal, first-class way to use the user's own **subscriptions** (Claude Code with Claude Pro/Max, Codex CLI with ChatGPT, Gemini CLI, OpenCode, etc.) is to **run the official CLI inside our terminal**, logged in with the user's own credentials, exactly as Warp and Conductor do.
- "New external agent" menu: detect installed CLIs on PATH (`claude`, `codex`, `gemini`, `opencode`, `copilot`, `aider`, …) and launch each in a dedicated pane with a recognizable tab icon.
- **Rendering quality is a headline feature:** test the flicker-prone cases (streaming responses, resize while streaming, Windows ConPTY). Add a `docs/MANUAL_TEST_CHECKLIST.md` section for it.
- **Agent-state detection (v1, needed for the v2 hand-off):** a pluggable `AgentDetector` per CLI reads the output stream (ANSI stripped) with configurable regex patterns. It detects states such as `working`, `waiting_for_input`, `waiting_for_approval`, and `limit_reached` (with an optional parsed reset time). These states drive OS notifications ("Claude Code needs approval in tab 3"). Patterns live in a config file so they can be updated without a release, and they're covered by fixture tests built from captured output.

---

## 6. Agent subsystem (our own agent)

### 6.1 Agent loop (`termflow-agent`)
- Classic tool-calling loop: build the request from the canonical transcript → `router.stream()` → render streamed events → when the model emits tool calls, run each through perms → snapshots (if mutating) → execute → append `tool_result` → loop until the model produces a final answer, the user cancels, or a step limit is hit (default 50 steps, configurable).
- Parallel tool calls: execute read-only tools concurrently; execute mutating tools one at a time, in order.
- **Modes:**
  - `build`: all tools available, subject to permissions.
  - `plan`: read-only. Write/edit/apply_patch/bash-with-side-effects are rejected at the perms layer, not just hidden. Produces a structured plan (`todo` list). The user can then "Execute plan in build mode", optionally with a different model profile ("plan with A, build with B").
- **System prompt** assembled from: base prompt (versioned file `prompts/system/base.md`), mode addendum, environment info (OS, shell, cwd, date, git status summary), project context (AGENTS.md / CLAUDE.md, Section 6.5), tool descriptions, and — after a model switch — the hand-off note (Section 7.7).
- **Slash commands:** `/plan`, `/build`, `/model`, `/profile`, `/compact`, `/undo`, `/redo`, `/clear` (new session, old one kept), `/cost`, `/attach` (last block), `/init` (generate an AGENTS.md by analyzing the repo), `/help`. User-defined commands from markdown files in `.termflow/commands/*.md` and `~/.config/termflow/commands/*.md` with `$ARGUMENTS` substitution (v1).
- **@-mentions** in the chat input: `@file`, `@folder`, `@block` (pick a terminal block), `@diff` (current git diff), `@url` (v1).
- **Cancellation:** Esc stops the current generation and any running tool. Keep the partial assistant text as text only and record the cancellation in the transcript.

### 6.2 Built-in tools (`termflow-tools`)
Each tool has: name, JSON schema for its arguments, `is_mutating`, `default_permission`, a concise description written for LLMs, and a timeout.

| Tool | Behaviour |
|---|---|
| `read` | Read a file (optionally a line range), with line numbers. Refuse binary files, warn on huge ones (show the first N lines plus the size). Respect ignore rules. |
| `write` | Create or overwrite a file (mutating, snapshot first). |
| `edit` | Exact string replacement (`old_string` must be unique unless `replace_all`), like Claude Code's Edit. Fail loudly if there's no match or more than one. |
| `apply_patch` | Apply a multi-file patch in a simple, robust format (unified diff or a V4A-style "*** Begin Patch" format; choose one, document it, and fuzz-test the parser). |
| `bash` | Run a command in a **separate non-interactive PTY or process**, not the user's visible terminal, with cwd = workspace, timeout (default 120 s, max 10 min), output cap (e.g. 30 KB tail + head), and exit code. Option `background: true` for dev servers (returns a handle; `bash_output`/`bash_kill` companion tools). Windows: run via pwsh by default, or Git Bash if configured. |
| `grep` | ripgrep-style search (`ignore` + `grep` crates), regex, glob filter, context lines, max results. |
| `glob` | File discovery by pattern, sorted by mtime. |
| `ls` | Tree listing with depth limit, respecting ignore rules. |
| `webfetch` | Fetch a URL, convert HTML→markdown, cap size. Asks permission by default. |
| `todo` | Read and write the session's todo list (shown in the UI). |
| `attach_block` | Give the agent the output of a terminal block by id (the user can also attach manually). |
| `outline` (v1) | tree-sitter symbol outline of a file. |
| `lsp_diagnostics` (v1) | Diagnostics for files the agent just edited (Section 6.7). |
| `task` (v1) | Launch a subagent (Section 6.6). |

Tool results are stored in full in the database. What gets sent to the model may be truncated, with an explicit note: "[truncated: 18,442 chars omitted; use read with a line range]".

### 6.3 Diff review
- Every mutation produces a `ChangeSet` (files, hunks, before/after hashes) that appears in the **Review panel**.
- Two review modes (setting, per session):
  - **Auto-apply + review after:** changes land on disk immediately (snapshotted); the user can revert per file or hunk.
  - **Approve before apply:** the tool call waits until the user accepts or rejects each file or hunk. Rejections go back to the model as tool errors with the user's optional comment.
- Side-by-side and unified views, syntax highlighting, word-level diff, keyboard navigation (j/k, a = accept, r = reject).
- Each change set links back to the plan step and the tool call that produced it ("why was this changed?").

### 6.4 Permissions (`termflow-perms`)
- Rules are **ordered; the last matching rule wins**. Effects: `allow`, `ask`, `deny`. A configured `deny` can **never** be overridden by a saved "always allow" approval.
- Rule matchers: tool name, argument patterns (glob for paths, prefix/regex for bash commands), and workspace scope.
- Example defaults (safe by default):
  ```toml
  [[permissions]]  tool = "*"        effect = "ask"
  [[permissions]]  tool = "read"     effect = "allow"
  [[permissions]]  tool = "grep"     effect = "allow"
  [[permissions]]  tool = "glob"     effect = "allow"
  [[permissions]]  tool = "ls"       effect = "allow"
  [[permissions]]  tool = "todo"     effect = "allow"
  [[permissions]]  tool = "bash"  command = ["git status*", "git diff*", "git log*", "ls*", "cat*", "cargo check*", "cargo test*", "pnpm test*", "npm test*"]  effect = "allow"
  [[permissions]]  tool = "bash"  command = ["rm -rf /*", "sudo *", "git push --force*", "curl * | sh", "*Remove-Item -Recurse -Force C:\\*"]  effect = "deny"
  [[permissions]]  tool = ["write","edit","apply_patch"]  path = ["**/.env*", "**/*.pem", "**/id_*"]  effect = "deny"
  [[permissions]]  tool = ["write","edit","apply_patch"]  path_outside_workspace = true  effect = "deny"
  ```
- Approval prompt UI: shows the exact command or diff, with buttons: Allow once / Allow for this session / Always allow (writes a rule to project config, shown to the user) / Deny / Deny with feedback.
- Autonomy presets: **Cautious** (ask for everything mutating), **Balanced** (default above), **Autonomous** (allow everything inside the workspace except the deny list). Deny rules are always active.

### 6.5 Project context (`termflow-context`)
- Discover and load, from the workspace root up to the git root: `AGENTS.md`, `CLAUDE.md`, `.termflow/rules/*.md`, and the global `~/.config/termflow/AGENTS.md`. Show which files are loaded in the session header.
- **Skills (v1):** load `SKILL.md`-based skills from `.termflow/skills/`, `~/.config/termflow/skills/`, **and `.claude/skills/` / `~/.claude/skills/`** (compatibility with Claude Code skills). Only the skill name and description go in the system prompt; the full body is loaded when the model calls a `skill` tool. This is progressive disclosure.
- Ignore rules: `.gitignore`, `.ignore`, `.termflowignore`. Secrets files are never read automatically.

### 6.6 Subagents and background tasks (v1)
- `task` tool: spawns a child agent with its own transcript, a restricted tool set (read-only by default), a **separate model route** (default `routes.background`, usually a cheaper model) and a focused prompt. It returns only a summary to the parent, which saves the parent's context and quota.
- The user can send any running agent to the background (Cmd/Ctrl+B). Background agents show in the Agents panel and send an OS notification when they finish or need approval.

### 6.7 LSP feedback (v1)
- Spawn language servers when they're installed: rust-analyzer, typescript-language-server, pyright, gopls, clangd, plus a configurable list. Use an LSP client crate **[verify: `lsp-types` + own JSON-RPC over stdio, or `async-lsp`]**.
- After each edit, collect diagnostics for the touched files (with a short debounce and timeout) and append them to the tool result, so the model fixes errors immediately.

### 6.8 Compaction
- Triggered when the context fitter (7.6) says the next request won't fit, or manually with `/compact`.
- Uses the **`routes.compaction` model** (must be explicitly configured; default = the session's current model).
- Produces a **structured checkpoint**, stored as a pinned message: `Goal`, `Current state`, `Decisions made`, `Files touched (with purpose)`, `Open problems / blockers`, `Next steps`, `Important snippets` (short). The pre-compaction turns stay in the database (append-only), but they're excluded from future requests.

### 6.9 Snapshots and undo/redo (`termflow-snapshots`)
- Per workspace, a **shadow git repository** under the app data dir (`<data>/snapshots/<workspace-hash>.git`) with `core.worktree` pointing at the workspace. **It never touches the user's `.git`.**
- Snapshot before every mutating tool call and at the end of every assistant turn. Each snapshot is linked to a transcript event id.
- `/undo` restores files **and** truncates the active conversation view to the previous turn. The truncated turns are kept in the database but marked as reverted, so `/redo` can bring them back.
- Respect .gitignore so `node_modules`/`target` aren't snapshotted. Configurable max snapshot size, plus garbage collection.

---

## 7. Model router with automatic fallback (the core differentiator)

This is the most important subsystem. Build it with TDD against the mock provider. It ships in the MVP (milestone M3) because the data model depends on it.

### 7.1 Canonical transcript (`termflow-types`)
```rust
pub struct Turn {
    pub id: TurnId,                 // ULID
    pub session_id: SessionId,
    pub role: Role,                 // System | User | Assistant | Tool
    pub parts: Vec<Part>,
    pub created_at: DateTime<Utc>,
    pub status: TurnStatus,         // Committed | Partial | Discarded | Reverted | Compacted
    pub served_by: Option<ServedBy>,// provider, model, credential_id, attempt_id (assistant turns)
}

pub enum Part {
    Text { text: String },
    Image { media_type: String, data_ref: BlobRef },
    ToolCall { call_id: CallId, name: String, args: serde_json::Value },
    ToolResult { call_id: CallId, content: Vec<ToolResultContent>, is_error: bool },
    Reasoning {                     // provider-native, opaque, stored verbatim
        family: ModelFamily,        // e.g. "anthropic", "openai", "google"
        model: String,
        credential_scope: String,   // account/org the signature is bound to
        native: serde_json::Value,  // exact block as returned (thinking + signature, encrypted_content, reasoning_details…)
        summary_text: Option<String>,
    },
    Checkpoint { summary: StructuredCheckpoint }, // from compaction
    HandoffNote { from: ServedBy, to: ServedBy, reason: ErrorClass, text: String },
}
```
- `CallId` is **ours** (ULID). An `id_map` table maps it to each provider's native tool-call id, because providers have different id formats and length limits.
- **Rendering per adapter:** each adapter turns the transcript into its wire format for every request. Switching models just means **re-rendering the same history for another target**; there's no state to migrate.
- Rules for rendering `Reasoning` parts:
  - Same family **and** same credential scope as the target → send the native block **unchanged and in its original position** (Anthropic requires assistant turns to be passed back exactly as returned; OpenRouter requires `reasoning_details` / signed items unchanged and in order).
  - Otherwise → **drop** the native block. Optionally include `summary_text` as plain text if the setting `carry_reasoning_summaries` is on (default off).
  - Anthropic silently drops thinking blocks it can't use, without billing them; still, never send another vendor's reasoning to it.

### 7.2 Provider adapters (`termflow-providers`)
Common trait (sketch):
```rust
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn wire_format(&self) -> WireFormat; // AnthropicMessages | OpenAIResponses | OpenAIChat | Gemini | Ollama
    async fn stream(&self, req: RenderedRequest, cred: &ResolvedCredential, cancel: CancellationToken)
        -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError>;
    fn classify(&self, err: &RawProviderError) -> ErrorClass;          // HTTP + body + SSE error events
    fn parse_rate_limit_headers(&self, h: &HeaderMap) -> RateLimitSnapshot;
    fn render(&self, transcript: &[Turn], ctx: &RenderContext) -> Result<RenderedRequest, RenderError>;
    async fn count_tokens(&self, req: &RenderedRequest) -> Option<u32>; // if the provider supports it
}

pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta { text: Option<String>, native_fragment: serde_json::Value },
    ToolCallStart { native_id: String, name: String },
    ToolCallArgsDelta { native_id: String, json_fragment: String },
    ToolCallEnd { native_id: String },
    Usage(Usage),                // input, output, cache_write, cache_read, reasoning tokens
    RateLimit(RateLimitSnapshot),
    Done { stop_reason: StopReason },
}
```
Adapters to implement:
- **MVP:** `anthropic` (Messages API, streaming, tools, extended thinking, prompt caching with `cache_control` breakpoints on system + tools + rolling last-user), `openai_responses` (Responses API, streaming, tools, reasoning items with `encrypted_content`), `openai_compatible` (Chat Completions, used for OpenRouter, Vercel AI Gateway, Groq, DeepSeek, LM Studio, vLLM, any custom endpoint), `ollama` (native API or its OpenAI-compatible endpoint).
- **v1:** dedicated `gemini` adapter, dedicated `openrouter` adapter (sends `models: [...]` and `provider: {...}` routing preferences and reads back the served `model`), `vercel_gateway` adapter (reads `modelAttempts`).
- Custom endpoints: the user can add any base URL and pick its wire format (OpenAI Chat, OpenAI Responses or Anthropic Messages), as Warp allows.

### 7.3 Credentials
```rust
pub enum CredentialKind {
    ApiKey { secret: SecretRef },                       // stored in OS keychain
    Local,                                              // Ollama / LM Studio, no secret
    SanctionedOAuth { provider: String, token: SecretRef }, // ONLY officially permitted third-party logins (Section 9). Off in MVP.
}
```
- Multiple credentials per provider are allowed (e.g., personal key + work key). Each has a label, optional monthly budget, and optional known reset policy.
- Connection test button: a cheap request (e.g., list models) shows ✓, or the classified error.

### 7.4 Error classification
A 429 can mean "wait 5 seconds" **or** "quota gone until next month"; **status codes alone are not enough**. Classify on HTTP status **+ response body error codes + SSE error events (errors can arrive mid-stream after an HTTP 200)**.

```rust
pub enum ErrorClass {
    TransientRateLimit { retry_after: Option<Duration>, reset_at: Option<DateTime<Utc>> },
    QuotaExhausted { reset_at: Option<DateTime<Utc>> },   // never retry same target
    Overloaded,                                            // 5xx, 529, 503, timeouts, connection errors
    ContextOverflow { limit: Option<u32> },
    Auth,                                                  // 401/403 → disable credential, prompt re-auth
    ContentPolicy,                                         // refusal/moderation → different vendor
    BadRequest { recoverable_by_stripping_native: bool },  // e.g. invalid thinking signature
    Cancelled,
    Unknown,
}
```

Known signals to encode (put each in a fixture test):

| Class | Signals |
|---|---|
| `TransientRateLimit` | HTTP 429 with `retry-after`; Anthropic `ramp_limit_exceeded`; OpenAI 429 `rate_limit_exceeded` / `slow_down`; rate-limit `*-remaining: 0` with a near `*-reset` |
| `QuotaExhausted` | Anthropic 429 `rate_limit_error` with `error.details.error_code = "enforced_spend_limit_reached"` and **no** `retry-after` (resets 00:00 UTC on the 1st of next month); Anthropic **400** "You have reached your specified API usage limits"; OpenAI `error.code` ∈ {`credit_balance_exhausted`, `organization_spend_limit_exceeded`, `project_spend_limit_exceeded`, `insufficient_quota`} (note: `error.type` may still say `insufficient_quota`); HTTP 402 / `billing_error`; Gemini `RESOURCE_EXHAUSTED` with quota details **[verify]** |
| `Overloaded` | 500, 502, 503 (`server_is_overloaded`), 504, Anthropic 529 `overloaded_error`, connect/read timeouts, TLS/connection reset, stream stalled > N s with no events |
| `ContextOverflow` | 413 `request_too_large`; 400 whose message mentions prompt too long / context length / maximum context **[verify exact strings; make patterns configurable]** |
| `Auth` | 401 `authentication_error`, 403 `permission_error` |
| `ContentPolicy` | refusal stop reasons, moderation errors |
| `BadRequest` | other 400s; if the message references thinking/signature/reasoning, mark `recoverable_by_stripping_native` |

- **Unknown error strings must not crash anything:** they map to `Unknown`, which is treated like `Overloaded` (retry once, then switch). They're also logged with the redacted body, so new patterns can be added.
- Classifier patterns live in a data file (`router/error_patterns.toml`) **and** in code defaults, so they can be patched without a code change.

### 7.5 Health, circuit breaker and budget
- **HealthRegistry** keyed by `(provider, model, credential)`: states `Closed` → `Open` (after `failure_threshold` failures within `window`, or immediately on `QuotaExhausted`/`Auth`) → `HalfOpen` after `cooldown` (minimum 30 s; for quota, `disabled_until = reset_at`) → probe with the next real request → `Closed` on success.
- **Proactive switching:** parse `anthropic-ratelimit-{requests,tokens,input-tokens,output-tokens}-{limit,remaining,reset}` and OpenAI `x-ratelimit-*` headers on every response. If the remaining tokens are below the estimated need of the next request, switch **before** failing. The user sees no error.
- **BudgetLedger:** per-session, per-day, per-month and per-credential spend, using catalog prices and the **actual served model** (providers bill for the model that served the request). It covers input, output, cache-write, cache-read and reasoning tokens. Budgets have a soft warning (e.g., 80%) and an optional hard stop. A hard stop pauses the agent with a clear dialog and never silently switches to a paid fallback.

### 7.6 Fallback chains, capability filter and context fitter
**Profiles** (user-editable in UI and TOML):
```toml
[profiles.coding-main]
chain = [
  "anthropic/<anthropic-flagship>@personal-key",
  "anthropic/<anthropic-mid>@personal-key",
  "openai/<openai-flagship>@openai-key",
  "openrouter/<open-coding-model>@openrouter-key",
  "ollama/<local-coder>",
]
on_context_overflow = ["google/<long-context-model>@google-key"]
on_content_policy   = ["openai/<openai-flagship>@openai-key"]
retry = { transient_max_wait_s = 10, overloaded_retries = 2, backoff = "exponential-jitter" }
hysteresis_turns = 3          # stay on the fallback at least K turns before switching back
switch_back = "when_healthy"  # "when_healthy" | "never" | "ask"
budget = { session_usd = 5.0, day_usd = 15.0, hard_stop = true }
require = { tools = true }    # capability requirements for every candidate

[routes]                      # scenario routing ("quota cruise mode")
background = "openrouter/<cheap-model>@openrouter-key"   # titles, subagents, search helpers
compaction = "openrouter/<cheap-model>@openrouter-key"
long_context = "google/<long-context-model>@google-key"
vision = "anthropic/<anthropic-mid>@personal-key"
```
**Selection algorithm per request:**
1. Start from the chain of the session's active profile, or the pinned model if the user pinned one.
2. **CapabilityFilter:** drop candidates lacking what this request needs: tool use, vision (if there are images), reasoning (if required), context window ≥ estimated tokens, max price.
3. Drop candidates whose health is `Open` or whose `disabled_until` is in the future, or whose budget is exhausted.
4. **ContextFitter:** estimate tokens (tokenizer approximation, or the provider's count-tokens endpoint when available). If the request doesn't fit the candidate, either pick a candidate from `on_context_overflow` or compact (6.8) and re-check.
5. Send. On error → classify → apply the action table:

| Class | Action |
|---|---|
| `TransientRateLimit` | If `retry_after` ≤ `transient_max_wait_s` → wait and retry the same target (show a countdown). Otherwise cool down until reset and move to the next candidate. |
| `QuotaExhausted` | **Never retry.** Set `disabled_until = reset_at` (or a configurable default like 1 h if unknown). Move to the next candidate immediately. |
| `Overloaded` | Retry up to `overloaded_retries` with exponential backoff and jitter, then move to the next candidate. Counts toward the circuit breaker. |
| `ContextOverflow` | Don't retry the same model. Use `on_context_overflow` or compact. |
| `Auth` | Disable the credential, show a re-auth banner, and move to the next candidate. |
| `ContentPolicy` | Move to `on_content_policy` (a different vendor). |
| `BadRequest{recoverable}` | Strip provider-native parts for that request and retry once; if it still fails, surface the error (don't blindly fall through the chain). |

6. **Hysteresis:** after a switch, stay on the fallback for at least `hysteresis_turns` turns, and only switch back once the primary is `Closed` again. This regains its prompt cache and avoids flapping.
7. If **all** candidates fail: pause the session (don't drop it), show a clear "All models unavailable" panel with each candidate's state and reset time, and offer: wait and auto-resume at the earliest reset, pick a model manually, or edit the profile.

### 7.7 Hand-off rules (how a switch keeps the session intact)
1. Native reasoning blocks are kept verbatim and only re-sent to the same family and credential scope (7.1).
2. For another vendor, render the history as plain text, tool calls and tool results.
3. **Mid-stream failure:** discard the partial assistant turn (mark it `Discarded`, keep it in the database). Optionally keep the plain text already streamed, but never a half-finished reasoning block or tool call. Re-send from the last **committed** turn. **Tools already executed are recorded as `ToolResult` and must never be executed again.**
4. If the history doesn't fit the new target, run the ContextFitter or compaction, keeping: the system prompt, the latest checkpoint, the last N turns, and every unresolved tool call/result pair.
5. Insert a short `HandoffNote` system/user-visible part: *"You are continuing a session started by another model. Current goal: … Plan state: … Last action: …"* (configurable on/off; default on for cross-vendor switches).
6. Expect a **prompt-cache miss** on every switch; record the estimated extra cost in the ledger.
7. Every attempt is recorded in an **attempt log** (`attempts` table: id, session, turn, provider, model, credential, started/ended, status code, ErrorClass, retry number, tokens, cost, latency, time-to-first-token).

### 7.8 UX for switching
- Non-blocking toast/inline banner: **"Anthropic `<model>` limited until 14:05 · continuing with `<model>` · [Pin this model] [Go back now] [Why?]"**.
- **"Why did it switch?" timeline**: a per-turn view of all attempts (target, status, class, wait time, cost).
- Model indicator in the chat header, showing the active profile, current model and health dots for each chain member.
- **Pin model** (disables auto-switch for this session) and **Auto** toggle.
- Live cost meter in the session header (tokens in/out/cache, $ this session, $ today), with a click for details.

### 7.9 Model catalog (`termflow-catalog`)
- Source of truth for model ids, families, context window, max output, capabilities (tools, vision, reasoning, caching) and prices: fetch **models.dev** (`api.json`) and/or the **OpenRouter models API** at startup (cached in SQLite, refreshed daily, works offline from cache). Users can override any field.
- Local models: query Ollama (`/api/tags`, `/api/show`) and LM Studio (`/v1/models`) for available models. Local models below a configurable size are flagged "tool use may be unreliable", because small local models often fail at tool calling.

---

## 8. Parallel agents, worktrees and external-agent hand-off

### 8.1 Worktrees (v1, `termflow-worktrees`)
- "New parallel session" creates a git worktree under `<repo>/.termflow/worktrees/<session-slug>` (or a configurable location outside the repo) on a new branch `termflow/<slug>`.
- Each session (our agent or a hosted external CLI) can be bound to a worktree. The Agents panel shows status (working / waiting for approval / done / error), model, cost, diff size, and elapsed time.
- **Unified review queue:** all finished parallel sessions show their diffs in one queue, with actions: open diff, merge into base branch (fast-forward or squash), create a PR (via `gh` CLI if installed), or discard the worktree.
- "Same prompt, N agents": run one prompt on several profiles in parallel worktrees and compare the results side by side.

### 8.2 ACP client (v1, `termflow-acp`)
- Implement the **Agent Client Protocol** client so ACP-compatible agents (Claude Code via its ACP adapter, Codex, Gemini CLI, OpenCode…) can run as structured agents with our native chat, diff review and permission UI, instead of only as terminal panes. **[verify the current ACP spec and Rust crates]**
- Install agents from the **ACP registry**.

### 8.3 Cross-agent relay (v2 — the signature feature)
When an `AgentDetector` (5.4) reports `limit_reached` for a hosted external CLI (e.g., Claude Code says it has hit its usage limit):
1. Show the banner: **"Claude Code reached its limit (resets ~15:00). Continue this task with: [Codex CLI] [OpenCode] [TermFlow agent: profile X] [Wait]"**.
2. Build a **hand-off package**: original task (from the first user message we captured, or the user's description), current `git diff` of the worktree, recent terminal transcript (ANSI-stripped, capped), files touched, any plan or todo found, and test status.
3. Launch the chosen agent on **the same worktree**, with the hand-off package as its first prompt (via ACP if available, otherwise by typing into the CLI's input or passing it as a CLI argument/file).
4. Record the relay in the session history.

---

## 9. Legal and terms-of-service constraints (hard rules)

1. **Do NOT implement OAuth or login with Claude Free/Pro/Max subscriptions** inside our agent, and do not reuse Claude Code's tokens. Since February 2026 Anthropic's consumer terms prohibit using those tokens in any other product; OpenCode removed this feature for that reason. Anthropic is supported **only through API keys**, or by **hosting the official Claude Code CLI** in our terminal.
2. **Never call private or undocumented endpoints** (e.g., ChatGPT's `backend-api`). Never spoof another client's headers or user agent.
3. **"Sign in with ChatGPT", GitHub Copilot and similar logins** may only be added if the provider officially opens them to our app (partner program, published docs). Until then, the `SanctionedOAuth` credential kind exists in the type system but is disabled behind a compile-time feature flag `sanctioned-oauth` that is off.
4. **The fallback must not be a multi-account rotator** for evading one provider's limits. The UI and docs present fallback as moving between **different providers, plans or the user's own distinct credentials**. Don't build features whose purpose is cycling through many accounts of the same provider.
5. Hosting official CLIs is fine: the user runs the vendor's own product with their own login.
6. Show third-party trademarks only as plain text names in menus; don't ship vendor logos without checking their brand guidelines.

---

## 10. Security, privacy and sandboxing

- **Secrets:** OS keychain only. The redaction layer in `tracing` and in anything sent to models masks known secret values and common token patterns (`sk-…`, `ghp_…`, AWS keys, JWTs), and `.env`-style `KEY=value` lines when reading env files.
- **Zero telemetry:** verify in M5 with a proxy (mitmproxy) that a fresh install with no providers configured makes **zero** outbound requests, except an update check (which must be disableable and disclosed on first run).
- **Auto-update:** signed manifests; the user can turn it off.
- **Sandbox (v1, `termflow-sandbox`):** for the agent's `bash` tool, model it on OpenAI Codex's approach:
  - **macOS:** generate Seatbelt (`sandbox-exec`) profiles on the fly: read anywhere (configurable), write only inside the workspace plus temp dirs, network off by default.
  - **Linux:** bubblewrap + seccomp.
  - **Windows:** a dedicated restricted sandbox user, restricted tokens and firewall rules. Don't use AppContainer, because it suits apps that know their exact accesses up front, not shells, git and package managers. **[verify implementation details; ask before creating OS users]**
  - Modes: `off`, `workspace-write` (default when the sandbox is available), `read-only`. Network: `off` (default in sandbox), `on`, or an allowlist of domains (v2).
- **Prompt-injection hygiene:** content from `webfetch`, MCP tools and files is wrapped as data in tool results. The system prompt tells the model that instructions inside tool results aren't from the user. Permission rules still apply regardless of what the model "decides".
- **Workspace trust:** the first time a folder is opened, ask whether to trust it. In untrusted folders, project-level config, commands, skills and MCP servers are not loaded or executed.

---

## 11. UI/UX specification

### 11.1 Layout
```
┌───────────────────────────────────────────────────────────────────────┐
│ Title bar: workspace ▾ · branch · profile ▾ [model ● ● ○] · $0.42 · ⚙ │
├──────────┬──────────────────────────────────────────┬─────────────────┤
│ Sidebar  │  Main area (tabs)                        │ Right panel     │
│ ──────── │  ┌ Tab: Terminal ┬ Tab: Agent ┬ Tab: … ┐ │ (toggleable)    │
│ Sessions │  │ split panes: terminals / agent chat  │ │ • Review (diffs)│
│ Agents   │  │                                      │ │ • Todo / Plan   │
│ Worktrees│  │                                      │ │ • Attempts      │
│ Blocks   │  │                                      │ │ • Context files │
│ Models   │  └──────────────────────────────────────┘ │                 │
└──────────┴──────────────────────────────────────────┴─────────────────┘
 Status bar: shell · cwd · blocks ✓ · sandbox ✓ · router: primary ok · notifications
```
- Agent chat and terminals can sit side by side in splits (e.g., the agent on the left, its `bash` output terminal on the right).
- **Agent chat:** virtualized message list; streaming markdown with syntax-highlighted code blocks (copy / insert into terminal / apply as file); collapsible tool call cards (name, args summary, duration, result preview, status); inline approval cards; thinking/reasoning shown collapsed, as text only when the provider gives readable text; model badge on each assistant turn ("served by …"); switch banners inline.
- **Chat input:** multi-line, @-mentions, slash commands with autocomplete, attach images (paste/drag), mode toggle (Plan/Build), profile selector, send (Enter) / newline (Shift+Enter), stop (Esc).
- **Settings:** a GUI for everything in TOML (providers and credentials, profiles and chains with drag-to-reorder, routes, budgets, permissions, terminal, appearance, keybindings, privacy, updates, experimental flags), plus "Open settings file".
- **Onboarding (first run):** choose a theme → choose a default shell → add at least one provider (API key or local model detection) or skip to terminal-only → build a first fallback profile from the configured providers with a "recommended order" → privacy statement (zero telemetry, update check toggle).

### 11.2 Visual design
- Modern, calm, dense but legible; inspired by Warp/Zed/Linear-level polish, but original. Dark theme default.
- Typography: system UI font for chrome; a monospace font for terminal and code (bundle one open-licensed font such as JetBrains Mono or Geist Mono **[check license]**, plus a user-selectable font).
- Design tokens in CSS variables (colors, spacing, radius, shadows, motion) so the theme can be swapped; respect OS reduced-motion and high-contrast settings.
- 60–120 fps animations only on transform/opacity; never animate layout during token streaming.
- Accessibility: full keyboard navigation, visible focus rings, ARIA roles on chat/list/tree components, screen-reader labels for icon buttons, WCAG AA contrast in default themes.

### 11.3 Keybindings (defaults; Cmd on macOS, Ctrl on Windows; all remappable)
| Action | Default |
|---|---|
| Command palette | Cmd/Ctrl+K |
| New terminal tab | Cmd/Ctrl+T |
| New agent session | Cmd/Ctrl+Shift+N |
| Split right / down | Cmd/Ctrl+D / Cmd/Ctrl+Shift+D |
| Close pane | Cmd/Ctrl+W |
| Toggle right panel | Cmd/Ctrl+\ |
| Attach last block to agent | Cmd/Ctrl+Shift+A |
| Send agent to background | Cmd/Ctrl+B |
| Plan/Build toggle | Shift+Tab |
| Stop generation | Esc |
| Undo agent change | Cmd/Ctrl+Z (in agent context) / `/undo` |
| Find | Cmd/Ctrl+F |

Watch out for conflicts with terminal programs: when a terminal pane is focused, only Cmd-based shortcuts are intercepted on macOS. On Windows, keep Ctrl+C/Ctrl+V terminal-aware (copy when there's a selection, otherwise SIGINT), as Windows Terminal does.

### 11.4 Internationalization
UI strings go through an i18n layer from day one (e.g., `@solid-primitives/i18n` **[verify]**). Ship **English** and **Spanish (es-ES)**. The agent's language follows the user's messages, not the UI language.

---

## 12. Milestones, scope and acceptance criteria

> Each milestone ends with green CI, updated docs, and a demo note. Never start the next milestone without my approval.

### M0 — Skeleton (MVP)
- Cargo workspace with all crates from Section 4 as compiling stubs (crates for v1/v2 can be empty with a `README.md` explaining their purpose).
- Tauri 2 app + SolidJS + Tailwind in `apps/desktop`, typed bridge generation (`specta`/`ts-rs`) working end to end with one example command and one example event.
- SQLite store with the migrations framework and a first migration (`sessions`, `turns`, `parts`, `attempts`, `blocks`, `costs`, `settings`).
- Config loading (global + project TOML) with JSON schema generation.
- `tracing` with redaction layer and rolling logs in the OS-appropriate data dir (`directories` crate).
- GitHub Actions: fmt, clippy `-D warnings`, tests (Rust + UI), build unsigned artifacts for macOS arm64, macOS x64 and Windows x64.
- `CLAUDE.md`, `AGENTS.md`, `docs/ARCHITECTURE.md`, `docs/MANUAL_TEST_CHECKLIST.md`.
- **Acceptance:** CI green on all three targets and produces a `.dmg` (both macOS archs) and `.msi`/NSIS `.exe`. The app opens a window showing the app version, read through the typed bridge. A Rust → UI event round-trip is shown on screen.

### M1 — Terminal (MVP)
- PTY, xterm.js WebGL, tabs, splits, shell selection, shell-integration injection, OSC 133/6973 → blocks, block header UI and actions, command palette, themes (dark/light + 4), font settings, find, links, terminal-aware copy/paste, hosting of external CLIs from the "New external agent" menu.
- **Acceptance:**
  - `vim`, `htop`/`btop`, `less`, `git log`, and `claude`/`codex` TUIs (if installed) render with no visible artifacts on macOS and on Windows (PowerShell 7 and Git Bash). Resizing while streaming works without corruption.
  - Each command in zsh/bash/fish/pwsh appears as a block with the correct exit code, duration and cwd.
  - A block can be copied as markdown and attached to an (empty) agent session.
  - Running `yes | head -n 2000000` keeps the UI responsive (no freeze longer than 100 ms; measure and report).
  - Idle memory with one terminal tab is reported in the summary.

### M2 — Basic agent (MVP)
- Provider connections UI with API keys stored in the keychain; adapters `anthropic`, `openai_responses`, `openai_compatible`, `ollama`; streaming chat UI; tools `read`, `write`, `edit`, `apply_patch`, `bash`, `grep`, `glob`, `ls`, `todo`, `attach_block`, `webfetch`; modes Build/Plan; slash commands from 6.1 (except user-defined ones); AGENTS.md/CLAUDE.md loading; @file/@block mentions; basic permission prompts (full rules in M4); cancellation.
- At this stage the agent calls the router with a **single-model profile** (the router passes through). The canonical transcript is used from day one.
- **Acceptance:**
  - In a sample Rust repo, the agent reads files, proposes a change, runs `cargo test` after approval, and reports the result.
  - In Plan mode any write/edit/apply_patch/mutating bash call is rejected at the perms layer (a unit test proves it).
  - The same conversation works against Anthropic, OpenAI and a local Ollama model, each with tool calls (tested against the mock provider, plus a manual test with real keys, documented).
  - Killing the app mid-stream and reopening it restores the session up to the last committed turn.

### M3 — Router with fallback (MVP, highest priority)
- Full Section 7: canonical rendering rules, id mapping, classifier with pattern file, health registry and circuit breaker, proactive header-based switching, budget ledger with catalog prices, capability filter, context fitter, profiles and chains, scenario routes, hysteresis, hand-off rules, attempt log, switch UX (banner, timeline, pin, health dots, cost meter), all-candidates-down panel with auto-resume.
- **Mock provider** (`tests/mock-provider`, axum): scriptable per-request responses — success streams, 429 with and without `retry-after`, Anthropic spend-limit body, 400 usage-limit body, 529, 503, 413, SSE stream that errors after N tokens, stream that stalls, invalid-signature 400, auth 401 — in Anthropic, OpenAI Responses and OpenAI Chat formats.
- **Acceptance (automated tests, all required):**
  - (a) A 429 `enforced_spend_limit_reached` with no `retry-after` → immediate switch with **no retry**; the target's `disabled_until` = 00:00 UTC on the 1st of next month.
  - (b) A 529 → exactly 2 retries with backoff, then a switch to the next candidate.
  - (c) An SSE error event after a 200 and 50 tokens → the partial turn is discarded and the session continues on the next model **without the user retyping anything**; the final transcript contains exactly one committed assistant turn for that step.
  - (d) A 413 → compaction or an `on_context_overflow` target is used; the request then succeeds.
  - (e) The ledger records the served model, tokens (including cache read/write) and cost for every attempt, including failed ones.
  - (f) A `tool_result` that already exists is **never re-executed** after a switch (assert the tool's execution counter).
  - (g) A 429 with `retry-after: 3` → waits about 3 s and retries the same target (with an injectable clock so the test is fast).
  - (h) Remaining tokens in the rate-limit headers below the next request's estimated size → proactive switch with no error shown.
  - (i) Hysteresis: after a switch, the primary isn't retried for K turns, then is used again once healthy.
  - (j) Anthropic thinking blocks are re-sent verbatim to the same family/credential and dropped for other vendors (snapshot tests of rendered requests).
  - (k) All candidates down → session paused, panel shows each reset time, auto-resume fires at the earliest reset (fake clock).
  - (l) Budget hard stop → agent paused with a dialog; no silent switch to another paid model.
- Property/fuzz tests for the transcript renderer, e.g. randomly generated tool-call histories always produce valid, balanced tool_call/tool_result pairs for every wire format.

### M4 — Trust (MVP)
- Diff review panel (both modes, file/hunk accept/reject, keyboard flow), full permission rules engine with presets and the approval UI, snapshots via shadow git repo with `/undo` `/redo`, structured compaction checkpoints, workspace trust prompt.
- **Acceptance:**
  - `/undo` restores files and conversation to the previous state and the user's `.git` is byte-identical before and after (test with a hash of `.git`).
  - A `deny` rule can't be bypassed by a saved "always allow" (unit test).
  - Rejecting a hunk sends a tool error back to the model with the user's comment.
  - Compaction produces all checkpoint sections, and the session continues coherently (scripted mock test).

### M5 — Packaging (MVP release)
- Updater (with disable toggle), complete settings UI, onboarding flow, zero-telemetry verification, crash-log capture (local only), installers, `docs/INSTALL.md` covering unsigned builds (macOS Gatekeeper: right-click → Open / `xattr -dr com.apple.quarantine`; Windows SmartScreen: "More info → Run anyway"). Note in the docs that public distribution later needs an Apple Developer ID (signing + notarization) and Windows Authenticode signing; leave CI hooks ready but disabled.
- Performance pass: measure cold start, idle RAM and streaming frame times on macOS and Windows; write the results to `docs/PERFORMANCE.md`.
- **Acceptance:**
  - Clean install, onboarding and first agent task work on macOS arm64, macOS x64 and Windows x64 (manual checklist).
  - With no providers configured and the update check off, mitmproxy shows zero outbound requests during a 10-minute session.
  - Performance targets from 1.4 are met, or the gaps are documented with a plan.

### v1 (post-MVP; split into milestones V1.1… when we get there)
Worktrees and parallel sessions; Agents panel and OS notifications; unified review queue and "same prompt, N agents"; MCP client (stdio + Streamable HTTP) with a visual manager (add/test/enable per project, tool allowlist); Skills (including `.claude/skills`); user-defined slash commands; subagents and background tasks; scenario routes UI; global quota dashboard (all providers, budgets, reset times); **pre-run cost estimate for a plan** (estimated tokens × price for each profile); LSP feedback; sandbox; IDE-style input editor; command autocompletion and rich searchable history; workflows (parametrized saved commands); session restore (layout + cwd); theme import; ACP client and registry; `termflow-daemon` (local HTTP+WS API with token auth bound to 127.0.0.1) and the headless `termflow` CLI (`termflow run "<task>" --profile X --worktree`); agent-state detectors for external CLIs; "Open in VS Code/Zed/Cursor"; dedicated Gemini/OpenRouter/Vercel adapters.
- **Acceptance highlights:**
  - Three agents run in parallel in separate worktrees and are reviewed from one queue.
  - One stdio and one HTTP MCP server work.
  - Claude Code installs from the ACP registry and runs with our diff/permission UI.
  - With the sandbox network off, `curl https://example.com` fails inside the agent's bash but works in the user's terminal.
  - `termflow run` completes a task headless and exits with a proper code.

### v2
Cross-agent relay (8.3); review mode with test evidence attached to each diff and an "Explain this change" action; "full terminal use" (the agent drives REPLs and TUIs in a PTY it controls, reading the screen through `alacritty_terminal`'s grid); semantic code index (tree-sitter chunks + embeddings in `sqlite-vec`, embeddings from a configured provider or a local model); **Agent Plugins 1.0** packaging support (MCP + skills in a folder with `plugin.json`) **[verify spec]**; mobile remote control (pair a phone via QR code to the local daemon over an end-to-end-encrypted relay or LAN) **[design doc first]**; sanctioned logins if the providers open them; optional cloud/microVM runners; optional native GPUI client.
- **Acceptance highlight:** when the detector sees Claude Code's limit message in a terminal pane, the relay banner appears, and choosing Codex CLI launches it in the same worktree with the hand-off package as its first prompt.

---

## 13. Data model (SQLite, initial)

Write the migrations in `crates/termflow-store/migrations/`. Tables (expand as needed):
- `workspaces(id, path, trusted, created_at)`
- `sessions(id, workspace_id, title, kind[agent|terminal|external], profile_id, pinned_model, mode, worktree_path, status, created_at, updated_at)`
- `turns(id, session_id, role, status, served_provider, served_model, credential_id, attempt_id, created_at, seq)`, with a unique `(session_id, seq)` index
- `parts(id, turn_id, idx, kind, json)` — the full `Part` JSON; large blobs go in `blobs`
- `blobs(id, sha256, media_type, bytes)`
- `tool_calls(call_id, session_id, turn_id, name, args_json, status, started_at, finished_at, exec_count)`
- `id_map(call_id, provider, native_id)`
- `attempts(id, session_id, turn_id, provider, model, credential_id, started_at, ended_at, http_status, error_class, retry_no, ttft_ms, input_tokens, output_tokens, cache_write_tokens, cache_read_tokens, reasoning_tokens, cost_usd)`
- `health(provider, model, credential_id, state, failures, opened_at, disabled_until, last_error_class)`
- `rate_limits(provider, model, credential_id, kind, limit, remaining, reset_at, observed_at)`
- `budgets(scope, scope_id, period, limit_usd, spent_usd, period_start)`
- `blocks(id, terminal_session_id, command, cwd, git_branch, shell, exit_code, started_at, finished_at, output_excerpt)`
- `snapshots(id, workspace_id, session_id, turn_id, git_oid, created_at, reverted)`
- `catalog_models(id, provider, family, context_window, max_output, caps_json, price_json, source, fetched_at, overrides_json)`
- `permissions_log(id, session_id, tool, args_summary, decision, rule_id, decided_by, at)`

Use WAL, `foreign_keys=ON` and a single writer task. Reads use a connection pool. Every transcript event is committed before the UI is told about it.

---

## 14. Quality, testing and CI

### 14.1 Testing strategy
- **Unit tests** in every crate. The router and providers must have high coverage (target ≥ 85% lines in `termflow-router`, `termflow-providers`, `termflow-perms`, `termflow-snapshots`).
- **Recorded fixtures** in `tests/fixtures/http/` for every error signal in 7.4, in each wire format. Fixture files contain no real secrets; CI scans for that.
- **Mock provider** integration tests (M3 list).
- **Snapshot tests** (`insta`) for rendered provider requests and the system prompt assembly.
- **Property tests** (`proptest`) for transcript rendering, patch application and permission rule matching.
- **Fuzzing** (`cargo-fuzz`, optional in CI) for the OSC parser and the patch parser.
- **UI tests:** Vitest + Solid Testing Library for components; E2E with Tauri's WebDriver support (`tauri-driver`) **[verify; on macOS WebDriver support may be limited — then use Playwright against the Vite dev build with a mocked bridge]**.
- **Real-provider smoke tests:** a separate opt-in workflow that runs only with secrets present (`TERMFLOW_LIVE_TESTS=1`), never on pull requests from forks.

### 14.2 Code quality
- `#![deny(unsafe_op_in_unsafe_fn)]`, clippy pedantic where reasonable (configure the allowed lints in the workspace `Cargo.toml`), `cargo deny` for licenses/advisories, `cargo machete` for unused deps.
- TypeScript: `strict: true`, ESLint + Prettier (or Biome), no `any` in `src/bridge`.
- Conventional Commits, one milestone per branch, PR description with a checklist of acceptance criteria.

### 14.3 Check suite (run before declaring anything done)
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
pnpm -C apps/desktop/ui lint
pnpm -C apps/desktop/ui typecheck
pnpm -C apps/desktop/ui test
pnpm -C apps/desktop tauri build --debug   # at least on the current OS
```

### 14.4 Performance checks
- A startup benchmark binary (time to first window plus time to interactive), recorded in CI as an artifact.
- Memory snapshot after 60 s idle with one terminal and one agent session (macOS: `footprint`/`ps`; Windows: PowerShell `Get-Process`), logged in CI.
- A streaming benchmark: mock provider at 200 tokens/s for 60 s; log the UI's dropped frames (via `requestAnimationFrame` timing instrumentation in a test build).

---

## 15. Files you must create in M0

### 15.1 `CLAUDE.md` (keep it under ~150 lines; update it every milestone)
It must contain:
- what the app is (2 lines);
- the stack table (short);
- the 10 architecture rules from Section 3, verbatim;
- the legal rules from Section 9, verbatim, under a "NEVER" heading;
- the workflow from Section 0 (plan → approval → TDD → implement → checks → docs);
- commands: dev (`pnpm -C apps/desktop tauri dev`), test, build, run the mock provider, regenerate the bindings;
- where things live (a crate map, one line each);
- the current milestone and a "known issues" list.

### 15.2 `AGENTS.md`
The same essentials as CLAUDE.md in tool-neutral wording (the open AGENTS.md standard), so other agents can contribute too.

### 15.3 `docs/ARCHITECTURE.md`
Diagrams (Mermaid) of: process model; crate dependency graph; event bus; agent loop; router decision flow; data model.

---

## 16. Definition of Done for the MVP (M0–M5)
- [ ] Installs and runs on macOS arm64, macOS x64 and Windows x64.
- [ ] Block-based terminal with tabs, splits, palette and themes; hosts Claude Code/Codex CLIs well.
- [ ] Own agent with Build/Plan modes, core tools, diff review, permission rules, undo/redo, compaction.
- [ ] Works with Anthropic API, OpenAI API, any OpenAI-compatible endpoint (OpenRouter, Vercel AI Gateway, custom) and Ollama/LM Studio.
- [ ] **Automatic, classified fallback that continues the same session across models and vendors**, with attempt timeline, live cost meter, budgets and auto-resume.
- [ ] Zero telemetry verified; secrets in the OS keychain only.
- [ ] All M3 acceptance tests (a)–(l) pass in CI.
- [ ] Docs: README, INSTALL (unsigned builds), ARCHITECTURE, PERFORMANCE, MANUAL_TEST_CHECKLIST, CHANGELOG.

---

## 17. Your first action

1. Read this document fully.
2. Reply with: (a) a summary of your understanding in at most 15 bullets; (b) every **[verify]** item you want to check first, and any that you already know are outdated, with proposed replacements; (c) open questions for me (license, confirmation of the GitHub owner/org for `flowlabs/termflow`, anything else you need); (d) the plan for **M0** as `docs/specs/M0.md`.
3. Wait for my approval before writing code.
