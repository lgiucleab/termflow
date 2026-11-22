# Changelog

All notable changes to this project. Format: Keep a Changelog; versions follow SemVer.

## [Unreleased]

### Added (M0 — Skeleton)
- Cargo workspace with all planned crates (v1/v2 crates are documented stubs).
- `termflow-types`: ULID ids, `AppInfo`, `AppEvent`, `ErrorClass`.
- `termflow-store`: SQLite (WAL, foreign keys), single writer thread, migration `0001_init`.
- `termflow-config`: global + project TOML merge, unknown keys rejected, JSON schema in `schemas/`.
- `termflow-secrets`: redaction of known secrets and common token patterns.
- `termflow-core`: composition root, event bus, rolling redacted logs.
- Desktop app (Tauri 2 + SolidJS + Tailwind v4) with typed bridge (tauri-specta), en / es-ES strings, version display and live core-event counter.
- CI: checks on Linux; unsigned installers for macOS arm64, macOS x64, Windows x64.
