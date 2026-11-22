# Manual test checklist

Things CI or the dev machine (macOS arm64) cannot verify. Tick per release.

## M0
- [ ] **macOS x64** (Intel Mac or Rosetta): open the `termflow-macos-x64` `.dmg` from CI, drag to Applications, right-click → Open. Window shows "Version 0.1.0" and the event counter increases every second.
- [ ] **Windows x64**: install the `.msi` and separately the NSIS `.exe` from CI (SmartScreen: More info → Run anyway). Same checks as above.
- [ ] Windows: logs are written under `%LOCALAPPDATA%\flowlabs\termflow\data\logs`.
