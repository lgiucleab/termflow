# 0001 — Chat-first, terminal secondary

- Date: 2026-11-22
- Status: accepted (owner)
- Supersedes: parts of BUILD_PROMPT §1.1 and §11.1 (layout). Everything else in the brief stands.

## Decision
TermFlow is a **chat-first** coding agent (closer to OpenCode than to Warp). The agent chat is the main screen, and the AI runs terminal commands as tools. The interactive terminal from M1 stays as a **secondary panel**: you can use it, the AI's environment matches it, and hosted CLIs (Claude Code, Codex…) still run there. It is no longer the app's home screen.

You open a **project folder** (like OpenCode or VS Code), and the chats in that window work in it. On start the app shows recent projects.

## Consequences
- **Layout (replaces §11.1):**
  - left sidebar: the project's chats (+ new chat)
  - main area: the selected chat
  - toggleable terminal panel (bottom or right) holding the M1 tabs/splits, starting in the project folder
  - The command palette and status bar stay.
- **Kept as built:**
  - M1 terminal, blocks, attach-to-agent, hosting of external CLIs. They live inside the panel.
  - M2a providers, transcript, router and chat.
  - M3 router/fallback, M4 trust, M5 packaging are unchanged.
- **Data:** sessions belong to a workspace (`sessions.workspace_id`, which already exists); `workspaces` lists recent projects.
- **New dependency:** `tauri-plugin-dialog` (native folder picker).
- **The brief's acceptance criteria still apply.** M1's terminal criteria are now about the panel.
