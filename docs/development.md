# Development

Back to the [README](../README.md).

## Layout

```
crates/core/   config, backend detection, providers, grab/paste
crates/cli/    mirzabenevis-cli  — headless; bind it to a key, or pipe text through it
src-tauri/     mirzabenevis      — tray app + popup window
ui/            the popup (plain HTML/CSS/JS, no bundler)
tests-e2e/     CLI end-to-end checks against mock servers and fake agent CLIs
flake.nix      dev shell, packages (default, cli), Home Manager module, checks
```

## Dev shell

```sh
direnv allow     # or: nix develop
cargo tauri dev  # the tray app + popup, reloads on change
cargo run -p mirzabenevis-cli -- doctor
```

Without Nix: a Rust toolchain, the
[Tauri prerequisites](https://tauri.app/start/prerequisites/#linux)
(`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev` on Debian/Ubuntu)
and `cargo install tauri-cli --version '^2'`.

## Pre-commit hooks

[prek](https://prek.j178.dev) runs the hooks in `.pre-commit-config.yaml`: whitespace,
YAML/TOML syntax, `cargo fmt --check`, and clippy on the core and CLI crates. Install them
once with `prek install`; run them all with `prek run --all-files`.

## Tests

| What | Command | Needs |
|---|---|---|
| Unit tests | `cargo test --workspace` | the dev shell (src-tauri links webkit) |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | the dev shell |
| CLI end to end | `cargo build -p mirzabenevis-cli && tests-e2e/run_all.sh` | `python3`; mocks only, no real AI calls; exits 1 on any failure |
| Nix package | `nix build .#default .#cli` and `nix flake check` | Nix |
| Debian package | `cargo tauri build --bundles deb --ci` | the dev shell |

CI (`.github/workflows/ci.yml`) runs all of these except the Nix build, and builds the
`.deb` on pushes to `main`. Pushing a `v*` tag makes a draft GitHub Release with the
`.deb` and AppImage (`release.yml`).

## Adding an action

Actions are plain config: add an `[[actions]]` entry to `config.example.toml` (it is the
built-in default config, compiled in). Pick a free `key` (see `RESERVED_KEYS` in
`crates/core/src/config.rs`), set `output` to `replace`, `show` or `clipboard`, and
mention it in the README key table if it gets a number.

## Adding a backend

All backends live in `crates/core/src/backends.rs`:

- an HTTP API with a key: an entry in `APIS` (env vars to look for, base URL, default
  model, OpenAI- or Anthropic-style transport);
- an agent CLI: an entry in `AGENTS` (binary name, how to pass the prompt);
- a local server: an entry in `LOCALS` (default ports, how to list its models).

Ids must be unique; `known_ids()` and `provider.order` pick them up. Add a detection test
in `backends_qa_tests.rs` and, for a new transport, an e2e case with the mock server.

## Releasing

1. Update `CHANGELOG.md` and the version in `Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Test the popup by hand on GNOME Wayland (grab, replace, undo).
3. `git tag v0.1.0 && git push --tags`, then publish the draft release.

## Roadmap

- [x] Settings window (`Ctrl+,` or ☰ → Settings…, see [popup.md](popup.md#popup-app))
- [ ] macOS / Windows grab & paste backends (core is already OS-agnostic)
