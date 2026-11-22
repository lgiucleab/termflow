# 0002 — One unified stream, monochrome UI

- Date: 2026-11-22
- Status: accepted (owner)
- Supersedes: the layout part of 0001 ("terminal secondary"). Projects (open a folder, recent list) from 0001 stay.

## Decision
TermFlow is **one coding tool**: an OpenCode-style agent and a Warp-style terminal in a single interface. Coding only.

- **Unified stream.** Each chat has one timeline and one input box.
  - A shell command runs in the chat's own shell and appears as a **command block** (command, exit code, duration, output).
  - Plain language goes to the **agent**, whose replies and tool cards appear in the same timeline.
  - Full-screen programs (vim, htop, less, Claude Code…) take over the pane while they run.
- **Monochrome design.** Black, white and greys for everything. Red and green only where they carry meaning: failed vs passed commands, diff lines. Dark by default, with a light variant.
- **Roadmap.** This is milestone **M2c — Unified UI**, before M3.

## Consequences
- Each chat owns a PTY shell in the project folder. The shell's own prompt is hidden, because TermFlow's input box replaces it; the output of each command goes into its block.
- This needs shell integration (OSC 133). Where it's missing (cmd, ssh to a host without integration), the chat falls back to a classic full terminal view.
- The agent's `bash` tool keeps running in its own captured process, shown as a tool card. That's safer and gives exact output, and it doesn't fight the user's shell.
- The separate terminal panel becomes an escape hatch (raw terminal of the chat's shell), no longer a second home.
- The M1 terminal stack (PTY, OSC parsing, blocks, xterm.js) is reused; it's mostly the UI that changes.
