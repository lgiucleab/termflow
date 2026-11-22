# M2d — OpenCode-style UI refresh (plan, awaiting approval)

Branch: `m2d-opencode-ui` (from `m3-router`). Status: **BUILT (2026-11-23)** — UI checks green; Tauri window check pending (manual checklist).

**Owner answers:** keep **Mono as the default** (decision 0002 stands; OpenCode's grey ramp is used, the accent is white/black, and Build/Plan tags are shown with greys and glyphs, not colour) · bundle **Inter + IBM Plex Mono** · M2d goes **before M3b** · Part B is approved as written.
Request (owner, 2026-11-22): take OpenCode's GUI look and its core features into TermFlow; refactor the whole UI to look modern and professional, like OpenCode.

Reference: OpenCode (`github.com/anomalyco/opencode`, **MIT**), packages `app` and `ui`, read from source. OpenCode's GUI uses the same stack as ours (SolidJS + Tailwind v4), so we can follow its design closely. We take **design values and layout**, not its code wholesale, its name, its logo or its wordmark. Credit goes in `THIRD_PARTY.md`. If we copy any file verbatim, it keeps its MIT header.

## Decisions needed before work starts
1. **Colour.** OpenCode isn't monochrome. It uses a neutral grey ramp plus a blue accent, and coloured tags for agents (Build blue, Plan pink). That contradicts decision 0002 ("Mono").
   - Proposal: add decision `0003`. The default theme becomes **"Flow"**: OpenCode's grey ramp and blue accent. Mono Dark/Light stay as themes.
   - Alternative: keep Mono as the default and take only the layout.
2. **Fonts.** OpenCode uses Inter (UI) and IBM Plex Mono (code). Both are under the OFL licence. Bundling them means a new top-level dependency (`@fontsource-variable/inter`, `@fontsource/ibm-plex-mono`). Alternatively we keep the system UI font and JetBrains Mono.
3. **Milestone order.** M3b (the switch UX) is next. Proposal: do M2d first, because M3b's banners and model chip would otherwise be built twice.
4. **No new UI library.** OpenCode uses Kobalte. We keep our own small components (popover, menu, dialog) on the existing stack.

## Part A — UI (this milestone)

### Tokens (`styles/tokens.css`, `features/themes/themes.ts`)
- Two-layer tokens, as OpenCode does:
  - **Primitive ramp:** grey 50–1200 (`#fff` → `#080808`), plus blue, green, red and yellow.
  - **Semantic tokens:** `bg-deep` (window), `bg-base` (panels), `layer-01..04`, `text-base/muted/faint`, `icon-*`, `border-muted/base/strong` (white/black at 8/10/20%), `overlay-hover/pressed`, `state-{success,warning,danger}-{fg,bg,border}`, `agent-build`/`agent-plan`, and diff add/delete surfaces.
- Type scale: 13px chrome (most text), 11–12px meta, 14/16/20px. Line height 20px.
- Radii: panel 10px, composer 12px, rows/buttons 6px, small 4px.
- Elevation instead of borders: panels get a raised shadow with a 0.5px ring.
- Motion: 120–150ms, `cubic-bezier(0.2,0,0,1)`. `prefers-reduced-motion` sets it to 0.
- `Theme.ui` grows to the semantic set. Existing themes (Mono, Nord, Gruvbox, Catppuccin) are re-mapped onto it. New themes are ported from palettes that are MIT/open, used by colour value only, with names shown as plain text.

### App shell
```
┌ titlebar 36px (bg-deep): project ▾ · branch · session tabs …            ⌘K  ⚙ ┐
│ ┌ sidebar 240 (resizable 136–640, ⌘⇧B) ┐ ┌ session panel (raised, r10) ──────┐ ┌ right panel ┐ │
│ │ sessions: 2-line rows                 │ │ header 48: title (inline rename) │ │ Review/Todo │ │
│ │ (title / status · time)               │ │ timeline, max 744px, centred     │ │ min 320     │ │
│ │ + New session                         │ │ … fade mask under composer       │ │ ⌘⇧R         │ │
│ └───────────────────────────────────────┘ │ ┌ composer (r12, raised) ──────┐  │ └─────────────┘ │
│                                           │ └──────────────────────────────┘  │                 │
│                                           ├ bottom panel: raw terminal (⌘J) ──┤                 │
└───────────────────────────────────────────────────────────────────────────────────────────────┘
```
- Panels float on `bg-deep` with 8px gutters.
- The current raw-terminal escape hatch becomes OpenCode's **bottom panel** (⌘J, resizable 120px–70%).
- The right panel ships with **Todo** (agent todos) and **Context** (attached blocks and files). **Review** waits for M4's diff review; the slot exists already.
- Rows: hover uses `overlay-hover`, active uses `overlay-pressed` plus `text-base`. A status dot shows working, attention or failed.

### Timeline (keeps decision 0002's unified stream)
- **User turn:** a quiet `layer-01` bubble, right-aligned text block, 13px.
- **Assistant:** plain markdown on the panel, served-by model badge in faint meta.
- **Thinking:** collapsed row "Thinking · 4s", expands to faint text.
- **Tool rows:** one 28px line each (icon · tool · args summary · status · duration), expanding to output or diff. Consecutive reads, greps and globs are grouped ("Explored 5 files").
- **Command blocks:** same row anatomy (`$ cmd` · exit badge · duration), with output below. The live xterm/frozen-HTML behaviour is unchanged.
- **Diffs:** add/delete surfaces, +/- counts in state colours.
- **Permission requests:** a **dock card above the composer** (warning icon, tool, args, patterns; buttons Deny / Allow always / Allow once). This replaces the inline approval card.
- **Errors and switch notices:** a "notice" row with the state-danger or warning surface. M3b will reuse it.
- **Timeline density setting:** Everything / Compact / Messages only.

### Composer
- Raised card, radius 12, max 720px.
- Auto-growing editor up to 180px, with fade masks.
- Faint placeholder: "Ask anything, run a command… / for commands, @ for files".
- Toolbar (44px), left to right:
  - `+` (attach, commands)
  - **agent select** (Build/Plan, coloured dot, Shift+Tab)
  - **model chip** (provider faint · model; opens the model picker)
  - reasoning toggle
  - spacer
  - mode glyph (`$`/`✦`, from `classify`)
  - submit button: arrow-up, stop while running, ↵ in shell mode
- **Todo pill** above the composer when the agent has todos.
- The `/` and `@` menus become one suggestion popover above the composer (rows 28px, radius 6).

### Palette, settings, home
- **Palette** (⌘K / ⌘⇧P): 36px search field, grouped results, 36px rows, shortcut hints in meta text.
- **Settings:** a full-screen page, not a modal. Left nav, 720px content. Pages: General, Appearance (theme, fonts, density), Providers (today's ProvidersDialog moves here), Shortcuts (read-only list for now), About.
- **Start screen:** a centred raised panel with the recent-projects list (40px rows) and an Open folder action.
- **New session:** a centred composer with a project · branch row under it. Small TermFlow wordmark (text only).

### Keybindings
Add ⌘⇧B (sidebar), ⌘⇧R (right panel), ⌘J (bottom terminal; replaces Cmd+J raw), ⌘, (settings) and ⌘⇧P (palette alias). Existing bindings are kept.

### Tests (TDD where there's logic)
- Theme mapping: every theme defines every semantic token, and text/bg pairs meet AA contrast (`themes.test.ts`).
- Tool-row grouping (a pure function).
- Panel resize clamping.
- Keybinding table.
- Palette grouping.
- Existing classify, splitter and slash tests stay green.
- Visual checks are manual; they go in `MANUAL_TEST_CHECKLIST.md` with before/after screenshots in the PR.

### Not in M2d
- Horizontal browser-style session tabs. We keep the sidebar (OpenCode's "vertical" layout); it fits a project-scoped tool better.
- Voice, QR pairing, and remote servers.
- **Session share links.** These conflict with zero telemetry, and there's no cloud.

## Part B — OpenCode core features → TermFlow

| OpenCode feature | TermFlow today | Where it lands |
|---|---|---|
| Build / Plan agents (Tab) | Plan mode in `termflow-perms`, Shift+Tab | M2d: agent select in the composer |
| Model switching per session | model picker | M2d: model chip; M3b: fallback chain in the chip |
| Permissions (allow/ask/deny, "always") | perms rules + approval card | M2d: dock UI; M4: presets |
| Todos | `TodoTool` | M2d: todo pill + right panel |
| `/init` → AGENTS.md | context discovery only | **M2d, small:** `/init` slash command that asks the agent to write AGENTS.md |
| Undo / redo / revert to message | — | M4 (snapshots) |
| Review panel (diffs) | — | M4 |
| Compact / summarise | — | M4 (compaction checkpoints) |
| Custom commands (`.opencode/commands/*.md`) | — | v1 (user slash commands); could start as M2e |
| Custom agents (markdown agents) | — | v1 |
| Subagents (general, explore) | — | v1 (6.6) |
| LSP diagnostics, formatters | — | v1 (6.7) |
| MCP | stub crate | v1 |
| Themes (~35) | 5 | M2d: token system + ~8 open palettes; theme import in v1 |
| Fork session | — | M4 next to snapshots (fork = copy transcript up to a turn) |
| Share | — | **Never.** Zero telemetry, no cloud |

Each of M4 and v1 gets its own spec as usual. This milestone gives every one of them a home in the new UI (a right-panel tab, a composer menu item, or a timeline row), so they won't need another redesign.

## Risks
- `ChatView.tsx` is 1,118 lines. The refresh splits it (`Timeline`, `TurnRow`, `ToolRow`, `BlockRow`, `Composer`, `PermissionDock`) without changing the behaviour of `chatShell`/`splitter`.
- Fonts add about 300 KB to the bundle.
- Turn rows hold xterm instances, so a virtualised list must keep the live block mounted.

## Commits (planned)
1. Tokens + themes re-map (+ tests).
2. Shell: titlebar, sidebar, panels, resize.
3. Timeline rows + grouping.
4. Composer + suggestions + permission dock + todo pill.
5. Palette, settings page, start/new-session screens.
6. Keybindings, `/init`, docs, CHANGELOG.

## As built (2026-11-23)
- **Tokens** (`styles/tokens.css`, `themes.ts`):
  - `Theme.ui` gained `deep`, `faint` and `warning`.
  - Hover, pressed and strong-border values, diff backgrounds and the three shadow levels are derived in CSS with `color-mix`, so every theme gets them for free.
  - Mono Dark now uses OpenCode's grey ramp (`#080808` window, `#161616` panels).
  - Tests check that every theme has every token, that terminal bg = panel bg, and Mono AA contrast.
- **Fonts:** `@fontsource-variable/inter` 5.3.0 and `@fontsource/ibm-plex-mono` 5.3.0 (both OFL). JetBrains Mono stays the terminal font.
- **Layout** (`stores/layout.ts`):
  - Sidebar 136–640px, right panel 320–640px, bottom terminal ≥120px and ≤70% of the session.
  - Sizes, open state and density are kept in localStorage (each access wrapped in try/catch).
  - `ResizeHandle` works with mouse and arrow keys.
- **Timeline:**
  - `group.ts` (tested) collapses runs of 2+ `read`/`grep`/`glob`/`ls` calls.
  - `ToolCard.tsx` renders rows, `ExploreRow` and `Diff`.
  - `PermissionDock.tsx` replaces the inline approval card.
  - Density hides thinking rows (Compact) or tool and thinking rows (Messages only) with CSS.
- **Composer:** the model/profile/credential controls moved from the old header into a popover on the model chip. `/model` opens it.
- **Settings** (`features/settings/Settings.tsx`):
  - Providers is now `ProvidersPanel`, and `setProvidersOpen(true)` opens that page.
  - The General page was dropped: nothing would be on it yet.
- **Keybindings:** `toggleSidebar`, `toggleRight` and `settings` added, plus ⌘⇧P for the palette. `ACTIONS` drives the Shortcuts page.
- **Credit:** OpenCode is credited in Settings › About and in the CHANGELOG. No OpenCode source file was copied, only design values and layout, so no extra licence file is needed.
- **Visual check:** done in a browser with a throwaway fake bridge (not committed), at dark/light, populated/empty and start screen. The real Tauri window is in the manual checklist.

### Known issues
- `/init` already existed (M2b), so no new work was needed there.
- Biome reports 5 `noDescendingSpecificity` warnings in `app.css` (warnings only; lint passes).
- In Mono Light the panel shadows are a little heavy. They could get lighter values per theme later.
- The title bar is a normal bar inside the native window frame (`data-tauri-drag-region` is set, but decorations are unchanged). A frameless window is a separate decision.
