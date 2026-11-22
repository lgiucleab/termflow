# M2c — Unified UI (plan, awaiting approval)

Branch: `m2c-unified` (from `m2b-agent`). Status: **PLAN — not started.** Decision: `docs/decisions/0002-unified-stream-mono.md`.

## Goal
One modern, minimal, black-and-white interface where **commands and the agent share one timeline**. You type in one input box: `cargo test` runs in the chat's shell as a command block; "why does add fail?" goes to the agent, which can see what you just ran. Full-screen programs take over the pane while they run. Everything built so far (PTY, shell integration, blocks, agent, tools, approvals) is reused; mostly the UI changes.

## Design

### Window
```
┌ sidebar ─────┐┌─ timeline (one per chat) ─────────────────────────┐
│ termflow ▾   ││ $ cargo test                        ✗ 101 · 4.2s │
│ ──────────── ││   test add ... FAILED                             │
│ ● fix tests  ││ ✦ why does add fail?                              │
│   api docs   ││   ◆ read src/lib.rs                          done │
│   …          ││   ◆ edit src/lib.rs          ┌ diff ┐ approve?   │
│ + New chat   ││   The bug is `a - b`; fixed to `a + b`.           │
│              ││ $ cargo test                        ✓ 0 · 1.1s   │
│              │├───────────────────────────────────────────────────┤
│ ⚙  ⌘K        ││ $ ❯ type a command, or ask the agent…   Build · M │
└──────────────┘└───────────────────────────────────────────────────┘
```
- **Sidebar**: project switcher, the project's chats, New chat, settings, and the palette. Same as M2b, restyled.
- **Timeline**: the chat's command blocks and agent turns, in time order, persisted. Past blocks reload with their plain-text output.
- **Input box**: one line, growing to multi-line. A mode glyph on the left shows where it goes: `$` (shell) or `✦` (agent).
  - **Auto-detect** (see below); **Ctrl+I / Cmd+I** locks the other mode.
  - Prefix `!` forces shell, `?` forces agent.
  - ↑/↓ cycle history per mode. Shift+Tab is Plan/Build. Esc stops the agent or the running command.
- **Raw terminal** (Cmd+J / Ctrl+Shift+J): the chat's shell as a classic full terminal. This is the escape hatch, and the fallback when shell integration is missing.

### Command blocks (Warp-style, on the M1 stack)
- Each chat owns one PTY shell in the project folder, with shell integration. The shell's own prompt is **hidden**; our input box replaces it.
- A small JS splitter reads the PTY byte stream:
  - prompt bytes (OSC 133 `A`…`C`) are dropped
  - bytes after `C` go to the **live block**
  - `D;exit` closes the block
- The **live block** is an xterm.js instance (colours, progress bars, `\r` updates all work).
  - While a command runs, focus moves into it: you can type into interactive prompts, and Ctrl+C reaches the program.
  - When it finishes, it's frozen to static HTML (`SerializeAddon.serializeAsHTML`) and the xterm is disposed, so only one xterm is alive per chat.
- **Alternate screen** (`?1049h`, used by vim/htop/less): the live block expands to fill the timeline area until `?1049l`, then collapses back into a normal block.
- **Degraded mode** (no OSC 133: cmd, nu, ssh to a host without integration): the chat shows the raw terminal with a small "blocks unavailable" note.
- Block actions on hover: copy command, copy output, ask the agent about this, re-run, collapse.

### Input auto-detection
Shell if the first word is a shell builtin (`cd`, `export`, `ls`…), an executable on PATH, or a path (`./x`, `~/x`), and the text doesn't read like a question or sentence (e.g. ends with `?`, or starts with how/why/what/can/please/fix/explain…). Otherwise agent.
- Executables come from one cached `PATH` scan in Rust (`list_executables`).
- The glyph updates as you type, so you always see where Enter will go.
- Pure function, unit-tested with a table of real examples (`git status` → shell; `fix the failing test` → agent; `make` → shell; `make the button blue` → agent).

### Agent ⇄ terminal
- Commands you ran since the agent's last reply are **auto-attached** as removable chips: the last 5 blocks, 10 KB of output each, ANSI stripped. "Why does add fail?" right after `cargo test` just works.
- The agent's own commands stay **tool cards** (separate captured process). That's safer, gives exact output, and doesn't disturb your shell. Either one can be re-run in the shell with one click.
- Blocks are linked to the chat: migration adds `blocks.chat_session_id`, and the terminal manager tags blocks with it.

### Visual system: Mono
- **Mono Dark** (default):
  - background `#000`, surfaces `#0a0a0a` / `#141414`, borders `#1f1f1f`
  - text `#fafafa`, muted `#8a8a8a`, accent = white
  - **only** semantic colours: success `#3fb950`, danger `#f85149` (exit badges, diff lines, error banners)
- **Mono Light**: inverse (`#fff` / `#0a0a0a`).
- Terminal ANSI palette: a desaturated version, so compiler errors and `ls` colours stay readable without breaking the monochrome look.
- Type: system UI font for chrome; **JetBrains Mono** for the timeline input, blocks and code.
- Styling: 13 px base, 1 px hairlines, 6 px radius, no shadows except floating menus, and motion only on opacity/transform (reduced-motion respected).
- Nord / Gruvbox / Catppuccin / High Contrast stay available in the palette; Mono is the default. WCAG AA contrast is checked for both Mono variants (text, muted text, focus rings).

## Tasks (TDD where practical)
1. Mono tokens + Mono Dark/Light themes, restyle of every existing surface: start screen, sidebar, cards, dialogs, palette, status bar.
2. `list_executables` + the input classifier, with a table-driven test.
3. JS stream splitter: OSC 133 + alt-screen detection on bytes, chunk-boundary safe. Property tests (any chunking gives the same split).
4. Chat shell sessions: PTY per chat (lazy, first use), cwd = project, tagged blocks (migration). Live block, freeze to HTML, alt-screen takeover, degraded fallback, raw terminal toggle.
5. Unified timeline: merge blocks and turns by time; past blocks from the DB; block actions; auto-attach recent blocks as chips.
6. Input box: mode glyph, auto-detect + override, per-mode history, focus hand-off to a running block, Esc/Ctrl+C semantics.
7. i18n (en, es-ES), checklist, docs, check suite, CI.

## Acceptance
- [ ] Typing `cargo test` runs it in the chat's shell and shows a block with command, exit code, duration and coloured output. Typing a question sends it to the agent. Both appear in one timeline and keep their order after restarting the app.
- [ ] `vim`, `htop`/`btop` and `less` take over the timeline area and return cleanly. An interactive prompt (`read x`) accepts typed input in the live block, and Ctrl+C stops a running command.
- [ ] After a failing command, asking the agent "why did that fail?" includes that block automatically (chip visible, removable).
- [ ] A chat on a shell without integration falls back to the raw terminal, with an indicator.
- [ ] Mono Dark is the default and Mono Light is available. Both pass WCAG AA for text and focus rings.
- [ ] The M1 terminal acceptance (blocks with exit code/duration/cwd, `yes | head -n 2000000` responsive) still holds in the new view.

## Risks / limits
- **Biggest UI change so far.** The xterm-per-live-block design keeps memory low, but Claude Code and other inline (non-alt-screen) TUIs need checking. If they misbehave, they open in the raw terminal automatically (planned fallback).
- **No shell tab-completion in the input box in M2c** (the shell's own completion lives in its line editor, which is hidden). History, file-path completion from the project file list, and the raw terminal cover it for now. Real completion is the v1 "command autocompletion" item.
- **Auto-detect will sometimes guess wrong.** The glyph shows the choice before Enter, and `!` / `?` / Ctrl+I override it.

## As built
- **One shell per chat.** It starts lazily the first time the chat is shown, in the project folder (120×24, with the prompt hidden). Blocks carry `chat_session_id` (migration `0005_chat_blocks`), so a chat's past commands come back from the DB after a restart (`chat_blocks`, last 500).
- **`StreamSplitter`** (`features/chat/splitter.ts`) runs on raw PTY bytes. It strips OSC 133/6973/7, reports marks and alt-screen switches (`?1049/1047/47`), passes every other OSC through, and is safe across chunk boundaries. Tests feed every chunking of the same stream.
- **Live block:** one xterm per running command, sized to its output (capped at 24 rows). On finish it is frozen to HTML with `serializeAsHTML`, sanitised with DOMPurify, and the xterm is disposed. Alt-screen programs take over the whole timeline area. Known inline TUIs (`claude`, `codex`, `gemini`, `opencode`, `aider`, `copilot`, `top`) open the classic terminal view instead.
- **Classic terminal** (Cmd/Ctrl+J, or the header's "Terminal" toggle): the same shell, replayed from a 4 MB raw log. If no prompt mark arrives within 6 s, the chat is "degraded" and only this view is used.
- **Input:** `classify()` uses the PATH executables (`shell_executables`), shell builtins, shell syntax and path-like tokens; `!` forces a command, `?` forces the agent, and Cmd/Ctrl+I or the glyph locks the mode. ↑/↓ history; `/` and `@` menus in agent mode only.
- **Auto-attach:** up to 5 blocks since the last agent turn, 10 KB each, shown as removable chips.
- **Removed:** the M1 tab/split workspace and the terminal panel (`features/layout`, `features/terminal/{controller,registry,TerminalPane,BlockHeader}`). Shortcuts are now palette, new chat, terminal, attach last block, next/previous chat.
- **Not done (moved to later):** separate history per mode, and typing into a running block from the main input. Input to a running command goes into its live block, which has focus.
