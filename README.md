# mirzabenevis

Select text in any app, press a key, get it fixed, rewritten or translated, pasted back in place.
Linux first, NixOS friendly, local-model friendly. Finds the AI you already have
(API keys, Claude Code, Ollama, ...) without configuration.

## Features

- Select text anywhere → press a hotkey → it is fixed, rewritten or translated and pasted back over the selection
- Auto-detects your AI: API keys (Anthropic, DeepSeek, OpenAI, Gemini, OpenRouter, ...), Claude Code, Codex, `llm`, Ollama, LM Studio, llama.cpp, vLLM, or any custom endpoint
- Inline word diff of what changed
- Type your own instruction, refine the result, and save it as a new chip with ★
- Modifiers: shorter, longer, warmer, formal, British, American
- Chat mode when nothing is selected; summaries and explanations rendered as Markdown
- History of runs with undo (paste the original back)
- A headless CLI to bind to a key or pipe text through
- Linux/NixOS first (Wayland and X11)

## Quick start (NixOS)

```sh
git clone https://github.com/ehsanirani/mirzabenevis && cd mirzabenevis
direnv allow            # or: nix develop
cargo run -p mirzabenevis-cli -- providers          # what AI backends were found
cargo run -p mirzabenevis-cli -- providers --check  # ...and does each one answer
echo "this are a test" | cargo run -p mirzabenevis-cli -- run fix --print
cargo tauri dev                                     # the tray app + popup
```

No config file is needed. `config init` writes a commented one to edit.

## GNOME Wayland setup

Grabbing the selection needs `wl-clipboard`; pasting back needs `ydotool` (`wtype` does not
work on GNOME). On NixOS:

```nix
programs.ydotool.enable = true;             # runs ydotoold
users.users.<you>.extraGroups = [ "ydotool" ];
environment.systemPackages = with pkgs; [ wl-clipboard ];
```

Log out and in after adding the group. Then add a custom shortcut in GNOME keyboard settings
that runs `mirzabenevis --popup` (or `mirzabenevis-cli run fix` for a one-key fix). The tray
icon needs the [AppIndicator](https://extensions.gnome.org/extension/615/appindicator-support/)
extension. Other compositors and X11: see [Grab and paste on Linux](docs/cli.md#grab-and-paste-on-linux).

## Popup keys

| Key | Does |
| --- | --- |
| `1`–`9` | Run that action (Fix grammar, Improve, Make concise, Translate, Summarise, ...) |
| `Enter` | Run the active action; with a result, Replace the selection |
| `/` | Type an instruction (refines the result; continues as a chat after a summary) |
| `D` / `X` | Toggle the word diff / explain what changed |
| `-` `+` `W` `F` `B` `A` | Toggle a modifier for the next run |
| `T` / `M` | Next translation language / backend picker |
| `U` | Undo the last replace |
| `C` / `R` / `E` | Copy / redo / edit the original |
| `Esc` | Cancel, or close |

All keys, chat mode and the ★ chips: [docs/popup.md](docs/popup.md).

## Documentation

- [Configuration](docs/configuration.md): backends, fallback, actions, modifiers, every setting
- [CLI and grab/paste](docs/cli.md): `mirzabenevis-cli`, history/undo, keybinds, paste internals
- [Popup](docs/popup.md): full key reference, chat, troubleshooting
- [Security and privacy](docs/security.md): agent-CLI sandboxing, history, clipboard
- [Development](docs/development.md): layout, tests, roadmap

## Status

Works on Linux (Wayland and X11). Windows and macOS are not supported yet: the core is
OS-agnostic, but grabbing the selection and pasting back are Linux-only.

## License

MIT, see [LICENSE](LICENSE).
