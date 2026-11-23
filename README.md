# TermFlow

An AI coding agent for the terminal and desktop, forked from [opencode](https://github.com/anomalyco/opencode).

## What's different

- **Automatic account switching.** Connect more than one account for the same provider
  (`termflow auth login` again with another key or sign-in). When the active account hits a
  rate limit or quota, TermFlow switches to the next saved account and retries right away.
  Once every account has been tried in a step, it falls back to the normal backoff.
  You can still switch by hand with `termflow auth switch`.
- TermFlow branding in the TUI, desktop app and CLI.
- Trimmed: opencode's hosted services, Salesforce app, Rust CLI rewrite, nix packaging and
  translated READMEs are removed.

## Install

CLI (macOS, Linux, Windows via Git Bash):

```bash
curl -fsSL https://github.com/lgiucleab/termflow/releases/latest/download/install | bash
```

Desktop app: download the installer for your OS from the
[latest release](https://github.com/lgiucleab/termflow/releases/latest)
(`.dmg` for Apple Silicon Macs, `.exe` for Windows x64, `.AppImage`/`.deb` for Linux x64).
The builds are unsigned, so macOS and Windows warn on first launch. The desktop app does not
update itself yet; `termflow upgrade` updates the CLI.

## Development

Requires [Bun](https://bun.sh) 1.4.4.

```bash
bun install
bun dev          # run the TUI from source
bun dev:desktop  # run the desktop app
```

## License

MIT. Based on opencode, Copyright (c) 2025 opencode. See [LICENSE](LICENSE).
