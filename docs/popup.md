# The popup

The tray app, every popup key, and chat mode. Back to the [README](../README.md).

## Popup app

```sh
cargo tauri dev        # inside the dev shell
```

If the popup window is blank (seen on NVIDIA and some Wayland setups), run it with
`WEBKIT_DISABLE_COMPOSITING_MODE=1`.

For how Replace pastes (and why it may not work in a terminal), see [Grab and paste on Linux](cli.md#grab-and-paste-on-linux).

It sits in the tray. Open the popup with the hotkey in `general.hotkey` (X11 only) or by
running `mirzabenevis --popup` (works everywhere, including Wayland, thanks to the
single-instance plugin). The popup grabs the selection into the read-mostly *Original* pane
(it grows with the text up to 40% of the window; the result gets the rest). Focus starts on
the action chips, so single keys work right away:

| Key | Does |
| --- | --- |
| `1`–`9` | Run that action on the original (built in: `1` Fix grammar, `2` Improve, `3` Make concise, `4` Translate, `5` Summarise, `6` Explain, `7` Key points (a short bullet list), `8` Make table (a Markdown table); a chip pressed mid-run cancels and reruns; Tab to a chip + Enter works too). Actions without a `key` are click-only chips. Non-Latin layouts use the physical key |
| `Enter` | Run the active action; once a result exists (and the original is unedited), **Replace** the selection with it (the footer says **Copy** when nothing can paste). For `output = "clipboard"` actions Enter copies and the popup stays open; for `output = "show"` actions (Summarise, Explain, Key points, Make table) nothing is replaced: the Enter hint disappears, `C` copies |
| `R` | Redo: run the active action again on the original (also resets a refine chain) |
| `C` / `Ctrl+C` | Copy the result (Ctrl+C copies your selection instead, if you made one) |
| `D` | Toggle the word diff (remembered per action; on by default for Fix grammar; no effect on shown answers, which are rendered as Markdown) |
| `E` | Edit the original (Enter runs once edited, `Shift+Enter` = newline, `Esc` back to the chips) |
| `/` | Type an instruction (Enter runs it on the original; with an unedited result it refines the *result* instead; after a shown answer such as a summary it continues as a **chat** about it; `Esc` back; the last 20 distinct instructions from the history are suggested). With nothing selected and an empty original, `/` is the chat box (see below) |
| `★` / `×` | `★` beside the instruction field saves the instruction as a new chip (an `[[actions]]` entry in `config.toml`, comments kept; click-only, `output = "replace"`). Hover one of your chips and click `×` to remove it again. Built-in chips have no `×`. Hovering any chip shows its prompt. **Right-click** a chip for a menu: edit its prompt in the instruction field (tweak, Enter runs it, `★` saves a variant), copy the prompt to the clipboard, run it, or remove it (your chips only). **Middle-click** = edit the prompt in the field directly |
| `Paste` / `Clear` | Buttons on the Original label: replace the text with the clipboard, or empty it |
| `☰` | Title-bar menu: About, Settings, open `config.toml` in your editor, reload config, quit |
| `Ctrl+,` | Open the settings (see below) |
| `-` `+` `W` `F` `B` `A` | Toggle a modifier (Shorter, Longer, Warmer, Formal, British, American; the row under the chips): its instruction is added to the next run, then it switches off. Clicking works too |
| `T` | Next translation language from `translate.targets` (the chip shows it: `4 Translate → de`; saved in the config) |
| `K` | Show the context field: a line such as "reply to my PI, keep it short" sent as `Context: …` with every run while it is set. Remembered for the rest of the day; `Esc`/`Enter` back (an empty field hides again) |
| `X` | With a rewritten result (not a shown answer such as a summary): ask the same backend what it changed and why (max 5 bullets), shown in the result pane; `X` again shows the result. The explanation is never pasted or copied: Enter/`C` still use the result |
| `U` | Undo: paste back the original of the last result that was actually pasted over a selection (from the history; select the replaced text in your app, open the popup, press `U`) |
| `M` | Open the backend picker (also: click the footer button): Auto, then what was found grouped as APIs / Agent CLIs / Local / Custom, each with its model in a text field. `↑`/`↓` + `Enter` chooses (saved as `provider.use`); type a model and press `Enter` in its field to save it (`provider.models.<id>`, like `config model`; empty = the default); `Esc` closes |
| `N` | In a chat: new chat (forgets the turns so far); no effect otherwise |
| `Ctrl+=` / `Ctrl+−` / `Ctrl+0` | Zoom in / out (80–140%) / reset; saved as `appearance.zoom` |
| `P` | Pin / unpin (pinned popups stay open when focus leaves; shown in the footer) |
| `Esc` | Cancel the running request (from anywhere); when idle, close (from an input: back to the chips) |

**Chat.** When the popup opens with nothing selected, the result pane becomes a chat: type in
the instruction field, `Enter` sends, answers stream in and are rendered as Markdown. Earlier
turns are sent along as a transcript (to the backend chosen in the picker, no fallback). `C`
copies the last answer, nothing is replaced, `N` starts over, `Esc` cancels/closes. Typing into
the original leaves the chat. Chats are not written to the history. Shown answers (Summarise,
Explain, Key points, Make table, `X`) and chat answers are rendered as Markdown (headings,
lists, **bold**, `code`, code blocks, tables); Copy always copies the raw text.

**Settings** (`Ctrl+,` or `☰` → Settings…) edit `config.toml` for you, comments kept. A
change is saved as soon as you leave the field (there is no Save button) and applies right
away; the dialog's status line says "Saved". A value the config would not accept (a key the
popup reserves or another action or modifier uses, a temperature outside 0–2, ...) is not
written: the field turns red with the reason. Tabs:

- **General**: the default action (Enter), replace automatically, paste by clipboard + Ctrl+V
  or by typing, history, desktop notifications (CLI), the language of explanations, and a
  reminder of the zoom keys.
- **Appearance**: theme (System / Light / Dark), zoom and font size (sliders), font family,
  accent colour and the window size; all apply at once (`[appearance]` in the config).
- **Actions**: every action's name, key, output, backend (blank = whatever the popup uses),
  model and prompt. `↑`/`↓` reorder them (the order of the chips), *+ Add action* adds one
  and puts the cursor in its prompt, built-in actions have *Reset to default* (name, key,
  prompt, output), your own have *Remove*. The first edit in a file without `[[actions]]`
  copies the built-in list into it, as `★` does (the same goes for modifiers).
- **Modifiers**: name, key and instruction of each; add, remove (yours) or reset (built in).
- **Translate**: the target language and the list `T` cycles through.
- **Advanced**: max tokens, temperature, timeout, longest input, the X11 hotkey (used after a
  restart), the config file's path, *Open config.toml* and *Reload*.

Each tab ends with *Reset this tab to defaults*, which removes that tab's settings from the
file so the defaults apply again. On Actions and Modifiers it is *Reset all to defaults*: it
deletes your own entries too, so it asks for a second click within 3 seconds. *Reset
everything* (Advanced, same second click) does that for every tab at once. API keys, custom
backends, the backend choice and the grab/paste commands are not on the page and no reset
touches them: edit the file for those.

The backend picker sits in the footer (hover for the full name). Replace pastes with
Ctrl+V when a key tool is available, else types the text; when neither exists (or
`auto_paste = false`) it copies the result, keeps the popup open and says
"Copied — Ctrl+V to insert". If pasting fails, the popup comes back with the error and the
result is on the clipboard.
