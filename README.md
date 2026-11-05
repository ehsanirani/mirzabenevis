# mirzabenevis

Select text in any app, press a key, get it fixed, rewritten or translated, pasted back in place.
Linux first, NixOS friendly, local-model friendly. One config file controls everything.

```
crates/core/   config, providers (OpenAI-compatible + Anthropic), grab/paste
crates/cli/    mirzabenevis-cli  — headless; bind it to a key, or pipe text through it
src-tauri/     mirzabenevis      — tray app + popup window
ui/            the popup (plain HTML/CSS/JS, no bundler)
```

## Quick start (NixOS)

```sh
cd ~/projects/mirzabenevis
direnv allow            # or: nix develop
cargo run -p mirzabenevis-cli -- config init
$EDITOR ~/.config/mirzabenevis/config.toml   # set provider/model
```

The default config points at Ollama (`http://localhost:11434/v1`, model `qwen2.5:7b`).
For any hosted provider change `[provider]`, and put the key in the env var named by
`api_key_env` (default `MIRZABENEVIS_API_KEY`).

Try it:

```sh
echo "this are a test" | cargo run -p mirzabenevis-cli -- run fix --print
```

## CLI

```
mirzabenevis-cli run [ACTION] [--print] [--copy] [--stdin] [--prompt "…"]
mirzabenevis-cli actions
mirzabenevis-cli grab
mirzabenevis-cli config path|init|show|model <name>|provider <openai|anthropic> [url]
```

`run` reads the primary selection (what you highlighted), sends it through the action,
and types the result back. With `--print` it writes to stdout; with piped input it reads stdin.

Bind it in your compositor, e.g. Hyprland:

```
bind = SUPER, G, exec, mirzabenevis-cli run fix
bind = SUPER SHIFT, G, exec, mirzabenevis --popup
```

Sway: `bindsym $mod+g exec mirzabenevis-cli run fix`.
GNOME/KDE: add a custom shortcut in keyboard settings with the same command.

## Popup app

```sh
cargo tauri dev        # inside the dev shell
```

It sits in the tray. Open the popup with the hotkey in `general.hotkey` (X11 only) or by
running `mirzabenevis --popup` (works everywhere, including Wayland, thanks to the
single-instance plugin). The popup grabs the selection, number keys run actions, Enter pastes,
Ctrl+C copies, Esc closes.

## Grab and paste on Linux

Reading the selection and typing text back is done by small external tools, listed in
`[linux]` in the config. The first one found in `PATH` wins:

| compositor | grab | paste |
|---|---|---|
| wlroots (Hyprland, Sway, river) | `wl-paste --primary` | `wtype` |
| KDE / GNOME Wayland | `wl-paste --primary` | `ydotool` (needs `ydotoold`; add `["ydotool","type","--file","-"]`) |
| X11 | `xclip -o -selection primary` | `xdotool type --file -` |

If no paste tool is available the result lands on the clipboard instead.

## Config

See `config.example.toml`. Add actions as `[[actions]]` tables; `{text}` in the prompt is
replaced by the selection. Reload from the tray menu after editing.

## Status / roadmap

- [x] CLI with streaming, primary-selection grab, type-back
- [x] Tauri tray + popup, single-instance `--popup` trigger
- [ ] Diff view in the popup
- [ ] Settings window (today: edit the TOML, "Reload config" in the tray)
- [ ] History of recent rewrites
- [ ] macOS / Windows grab & paste backends (core is already OS-agnostic)
- [ ] Nix package + NixOS module
