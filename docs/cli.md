# Command line and grab/paste

`mirzabenevis-cli` and how text is read from and pasted back into other apps. Back to the [README](../README.md).

## CLI

```
mirzabenevis-cli run [ACTION] [--print] [--copy] [--stdin] [--prompt "…"] [--backend ID] [--target LANG]
mirzabenevis-cli history [-n N]
mirzabenevis-cli undo
mirzabenevis-cli providers [--check] [--json]
mirzabenevis-cli actions
mirzabenevis-cli grab
mirzabenevis-cli config path|init|show|use <id|auto>|model <name> [--backend ID]
```

`run` reads the primary selection (what you highlighted), sends it through the action,
and pastes the result back (see [Grab and paste on Linux](#grab-and-paste-on-linux) below). With `--print` it writes to stdout; with piped input
it reads stdin. When nothing is selected it exits with status 1 and does nothing.
An action's `output` decides the default: `replace` pastes, `clipboard` copies, `show` prints
to stdout (and, from a keybind, shows it as a notification); `--print` and `--copy` win over
it. `--target de` sets the language for `{target}` (the Translate action) for this run.

Every successful run is kept in `$XDG_DATA_HOME/mirzabenevis/history.jsonl` (usually
`~/.local/share/mirzabenevis/`; at least the newest 500 runs). It holds your text and the
results as **plain text**: readable only by you (mode 600) but not encrypted, and it ends up
in your backups. Delete it whenever you like, or turn it off with `general.history = false`
(then `undo` and the popup's recent instructions have nothing to work with). `history`
prints the last N runs (default 10). `undo` pastes back the original of the last result
that was actually pasted over a selection (by `run` or the popup's Replace; copied or
shown results do not count): select the replaced text and run it.

Started from a keybind (no terminal), `run` shows a desktop notification via `notify-send`
when nothing is selected, when it fails (`Failed: …`) and when a fallback backend answered.
Turn this off with `general.notify = false`.

Bind it in your compositor, e.g. Hyprland:

```
bind = SUPER, G, exec, mirzabenevis-cli run fix
bind = SUPER SHIFT, G, exec, mirzabenevis --popup
```

Sway: `bindsym $mod+g exec mirzabenevis-cli run fix`.
GNOME/KDE: add a custom shortcut in keyboard settings with the same command.

## Grab and paste on Linux

Reading the selection and pasting text back is done by small external tools, listed in
`[linux]` in the config:

| compositor | grab | paste (Ctrl+V) |
|---|---|---|
| wlroots (Hyprland, Sway, river) | `wl-paste --primary` | `wtype` |
| GNOME / KDE Wayland | `wl-paste --primary` | `ydotool` (`wtype` never works on GNOME's Mutter) |
| X11 | `xclip -o -selection primary` | `xdotool` |

**Grab.** Only the primary selection (what is highlighted) is used. If nothing is
highlighted, nothing happens; set `linux.grab_fallback_clipboard = true` to use the
clipboard then, or `grab = "clipboard"` to always use it.

**Paste.** With `paste = "clipboard"` (default) the result is put on the clipboard, Ctrl+V
is pressed with the first of `paste_key_cmds` that works, and ~300 ms later your previous
clipboard text is put back (only text; an image on the clipboard is not restored). If no key
tool works it types the text with `paste_cmds` instead (`paste = "type"` does only this);
if that fails too the result is left on the clipboard. Each list is tried in order until one
command succeeds, skipping `xdotool` on Wayland and `wl-*` tools on X11 (the same goes
for `grab_cmds`). Typing is slower and can mangle non-ASCII text with some tools, so
Ctrl+V is preferred. Ctrl+V does nothing in most terminals (they paste with
Ctrl+Shift+V), and mirzabenevis cannot tell: if you mostly use it in a terminal, set
`paste = "type"`. The CLI also prints a pasted result on stderr so it is never lost.
A clipboard marked as a password (`x-kde-passwordManagerHint`, set by KeePassXC and
others) is not saved or restored.

On Wayland the clipboard belongs to the program that set it, so install `wl-clipboard`:
`wl-copy` keeps the result (and the restored clipboard) alive after `mirzabenevis-cli`
exits. On X11 the clipboard is set in-process: it lives long enough for the paste and the
restore, but whatever the CLI or the popup left on the clipboard (e.g. after `--copy`)
is gone once they let go of it, unless a clipboard manager takes it over.

GNOME Wayland on NixOS:

```nix
programs.ydotool.enable = true;             # runs ydotoold
users.users.<you>.extraGroups = [ "ydotool" ];
environment.systemPackages = with pkgs; [ wl-clipboard ];
```

Log out and in after adding the group. wlroots: `[ wl-clipboard wtype ]`; X11:
`[ xclip xdotool ]`. None of these tools are in the dev shell.
