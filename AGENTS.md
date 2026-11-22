# TermFlow — agent guide (AGENTS.md)

TermFlow (by Flow Labs) is a cross-platform desktop app: a block-based terminal, a provider-agnostic coding agent, and an in-app model router that keeps a session going across models when one hits a limit or fails.
The full product spec is `docs/BUILD_PROMPT.md` — it is the source of truth; read the relevant section before any milestone work.

## Stack
Rust stable (edition 2024, tokio) · Tauri 2 (core in-process) · SolidJS 1.9 + TypeScript strict + Vite + Tailwind v4 · xterm.js · portable-pty · rusqlite (WAL) · keyring · rmcp · tauri-specta (typed bridge) · pnpm · GitHub Actions.

## Architecture rules (non-negotiable)
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

## NEVER (legal)
1. **Do NOT implement OAuth or login with Claude Free/Pro/Max subscriptions** inside our agent, and do not reuse Claude Code's tokens. Since February 2026 Anthropic's consumer terms prohibit using those tokens in any other product; OpenCode removed this feature for that reason. Anthropic is supported **only through API keys**, or by **hosting the official Claude Code CLI** in our terminal.
2. **Never call private or undocumented endpoints** (e.g., ChatGPT's `backend-api`). Never spoof another client's headers or user agent.
3. **"Sign in with ChatGPT", GitHub Copilot and similar logins** may only be added if the provider officially opens them to our app (partner program, published docs). Until then, the `SanctionedOAuth` credential kind exists in the type system but is disabled behind a compile-time feature flag `sanctioned-oauth` that is off.
4. **The fallback must not be a multi-account rotator** for evading one provider's limits. The UI and docs present fallback as moving between **different providers, plans or the user's own distinct credentials**. Don't build features whose purpose is cycling through many accounts of the same provider.
5. Hosting official CLIs is fine: the user runs the vendor's own product with their own login.
6. Show third-party trademarks only as plain text names in menus; don't ship vendor logos without checking their brand guidelines.

## Workflow
Per milestone: write `docs/specs/<milestone>.md` → get owner approval → tests first (TDD) → implement in small commits on `m<N>-<short-name>` → run the check suite until green → update the spec + `CHANGELOG.md` → short summary (works / how to try / left / risks).
Ask before: new top-level dependency, stack or architecture changes, deleting user data, anything in the legal rules. Verify crate/package versions before pinning. Never invent model IDs (fixtures only).
Untestable-here behaviour goes in `docs/MANUAL_TEST_CHECKLIST.md`.

## Commands
- Dev app: `pnpm -C apps/desktop install && pnpm -C apps/desktop tauri dev`
- Rust checks: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo deny check`
- UI checks: `pnpm -C apps/desktop/ui lint && pnpm -C apps/desktop/ui typecheck && pnpm -C apps/desktop/ui test`
- Debug build: `pnpm -C apps/desktop tauri build --debug`
- Regenerate TS bridge: `cargo test -p termflow-desktop export_bindings` (also runs on every debug launch)
- Regenerate config schema: `cargo run -p termflow-config --bin gen-schema`
- Mock provider: arrives in M3 (`tests/mock-provider`).
- Dev harness: `TERMFLOW_DEV_AUTORUN='yes | head -n 2000000'` types a command into each new terminal (debug builds); frame times land in the log under `perf`.

## Where things live
- `crates/termflow-types` — ids, AppEvent, ErrorClass, transcript (M2+)
- `crates/termflow-config` — TOML load/merge, JSON schema (`schemas/`)
- `crates/termflow-store` — SQLite, migrations in `migrations/`, single writer thread
- `crates/termflow-secrets` — redaction (keychain in M2)
- `crates/termflow-core` — composition root, event bus, logging
- `crates/termflow-pty` — PTY spawn/IO, shell + CLI discovery
- `crates/termflow-shell-integ` — OSC scanner, block tracker, injection; scripts in `shell-integration/`
- `crates/termflow-terminal` — terminal sessions, batching, back-pressure, block persistence
- `crates/termflow-{providers,router,catalog}` — models and fallback (M2/M3)
- `crates/termflow-{agent,tools,perms,context,snapshots}` — agent (M2/M4)
- `crates/termflow-{mcp,acp,sandbox,worktrees,daemon}`, `apps/cli` — v1 stubs
- `apps/desktop/src-tauri` — Tauri glue only
- `apps/desktop/ui/src/bridge` — the only code that calls Tauri; `generated/` is tauri-specta output
- `apps/desktop/ui/src/features/terminal` — xterm controller (outlives components), block headers; `layout/` pure tab/split model

## Status
Current milestone: **M1 — Terminal** (`docs/specs/M1.md`). M0 done.
Known issues: see the spec's "Known issues" section.
