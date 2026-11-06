# Development

Back to the [README](../README.md).

## Layout

```
crates/core/   config, backend detection, providers, grab/paste
crates/cli/    mirzabenevis-cli  — headless; bind it to a key, or pipe text through it
src-tauri/     mirzabenevis      — tray app + popup window
ui/            the popup (plain HTML/CSS/JS, no bundler)
```

## Tests

```sh
cargo test --workspace
cargo build -p mirzabenevis-cli && tests-e2e/run_all.sh   # mocks only, no real AI calls
```

## Status / roadmap

- [x] CLI with streaming, primary-selection grab, type-back
- [x] Tauri tray + popup, single-instance `--popup` trigger
- [x] Automatic backend detection (APIs, agent CLIs, local servers) with fallback
- [x] Diff view in the popup
- [ ] Settings window (today: edit the TOML, "Reload config" in the tray)
- [x] History of recent rewrites (`history`, `undo`)
- [ ] macOS / Windows grab & paste backends (core is already OS-agnostic)
- [ ] Nix package + NixOS module
