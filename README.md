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

<!-- Screenshot: drop a popup screenshot at docs/img/popup.png and uncomment.
![The popup](docs/img/popup.png)
-->

## Install

NixOS, with this repo as the flake input `mirzabenevis`:

```nix
environment.systemPackages = [ inputs.mirzabenevis.packages.${pkgs.stdenv.hostPlatform.system}.default pkgs.wl-clipboard ];
programs.ydotool.enable = true;                  # pastes back on GNOME/KDE Wayland
users.users.<you>.extraGroups = [ "ydotool" ];
```

Home Manager module, `.deb` / AppImage, building from source and the GNOME tray
extension: [docs/install.md](docs/install.md). Then run `mirzabenevis-cli doctor`: it
checks the grab/paste tools and AI backends, and prints the exact line to bind
`mirzabenevis --popup` to a key on your desktop. No config file is needed;
`mirzabenevis-cli config init` writes a commented one to edit.

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
| `Ctrl+,` / `☰` | Settings (actions, modifiers, languages, ...) / menu |
| `C` / `R` / `E` | Copy / redo / edit the original |
| `Esc` | Cancel, or close |

All keys, chat mode and the ★ chips: [docs/popup.md](docs/popup.md).

## Documentation

- [Install](docs/install.md): NixOS, Home Manager, .deb/AppImage, from source
- [Configuration](docs/configuration.md): backends, fallback, actions, modifiers, every setting
- [CLI and grab/paste](docs/cli.md): `mirzabenevis-cli`, history/undo, keybinds, paste internals
- [Popup](docs/popup.md): full key reference, chat, troubleshooting
- [Security and privacy](docs/security.md): agent-CLI sandboxing, history, clipboard
- [Development](docs/development.md): dev shell, tests, adding a backend or action, releasing
- [Changelog](CHANGELOG.md)

## Status

Works on Linux (Wayland and X11). Windows and macOS are not supported yet: the core is
OS-agnostic, but grabbing the selection and pasting back are Linux-only.

## License

MIT, see [LICENSE](LICENSE).
