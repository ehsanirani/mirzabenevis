# Configuration

Backends, actions, modifiers and every other setting. Back to the [README](../README.md).

## AI backends (found automatically)

| kind | what is detected | default model |
|---|---|---|
| APIs | `ANTHROPIC_API_KEY`, `DEEPSEEK_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`/`GOOGLE_API_KEY`, `OPENROUTER_API_KEY`, `GROQ_API_KEY`, `MISTRAL_API_KEY`, `XAI_API_KEY`, `DASHSCOPE_API_KEY`, `TOGETHER_API_KEY`, `CEREBRAS_API_KEY`, `MOONSHOT_API_KEY` | a fast, cheap model per provider (e.g. `claude-haiku-5-5`, `deepseek-flash`, `gpt-6-luna`), with "thinking" switched off where possible |
| agent CLIs | `claude` (Claude Code: uses your Claude login, runs with all tools off), `codex`, `gemini`, `llm` | the CLI's own default (`haiku` for Claude Code) |
| local | Ollama, LM Studio, llama.cpp server, vLLM, Jan, when running | the first chat model installed |
| custom | `[provider.custom]`: any OpenAI-compatible or Anthropic URL, or any command | yours |

API keys are looked up in the environment, in your login shell (so keys exported in
`~/.zshrc`/`~/.bashrc` work when the app is started from the desktop; cached for 30 min
in `$XDG_RUNTIME_DIR`), in `llm keys` and in opencode's `auth.json`.

`provider.use = "auto"` takes the first one found (APIs, then agent CLIs, then local
servers; change with `provider.order`). If it fails before answering (e.g. a CLI that is
installed but not logged in), the next one **from the same vendor** is tried: Anthropic
API ↔ Claude Code, OpenAI ↔ Codex, one local server ↔ another. Your text is never sent to
a different company behind your back; when nothing of that vendor works, the error says
how many other backends were not tried. Pin one instead:

```sh
mirzabenevis-cli config use claude-code      # or deepseek, ollama, auto, ...
mirzabenevis-cli config model sonnet         # model of the backend in use
mirzabenevis-cli run fix --backend deepseek  # one-off
```

The popup has the same choice in its footer ([`M`, the backend picker](popup.md#popup-app)).

How agent CLIs (Claude Code, Codex, Gemini CLI) are sandboxed: see [security.md](security.md#agent-clis).

## Config

See [`config.example.toml`](../config.example.toml). Reload from the tray menu after editing (this also re-reads your
login shell for new API keys). Most of it can also be changed from the popup's
[settings](popup.md#popup-app) (`Ctrl+,`), which edit this file for you.

- **Actions** are `[[actions]]` tables. The selection is sent inside `<text>…</text>` tags
  (the system prompt tells the model to treat it as data, not instructions): at `{text}`
  in the prompt, or after the prompt when there is no `{text}`. Your `[[actions]]` replace
  the built-in list; duplicate `id`s or `key`s are an error when loading. Keys are
  case-insensitive. **Reserved** by the popup: `Enter` `Esc` `r` `c` `d` `p` `e` `/` `u` `t`
  `k` `x` `m` `n`; an action or modifier bound to one loses its key (a warning is logged, set
  `MIRZABENEVIS_LOG=warn` to see it) and stays clickable.
- **Per-action backend and model**: `backend = "claude-code"` makes that action use this
  backend whatever `provider.use`, the popup picker or `--backend` say; `model = "sonnet"`
  overrides the model (pair it with `backend`, since a model name is backend-specific).
- **Output**: `output = "replace"` (default), `"clipboard"` or `"show"` per action, see
  [CLI](cli.md#cli) and the [popup keys](popup.md#popup-app). The built-in Summarise, Explain, Key points and Make table
  only show; their answers are kept as the model wrote them (Markdown), not cleaned up.
- **Explanations**: `general.explain_in = ""` (default: the language of the text), e.g. `"fa"`
  or `"German"`, fills `{explain_in}` in prompts: the built-in Explain action and the popup's `X`
  answer in that language (the status after `X` shows it).
- **Your own chips**: the popup's `★` appends `[[actions]]` with `id` (the first three words as
  a slug), `name`, `prompt` and `output = "replace"`. Since your `[[actions]]` replace the
  built-in list, a file without any first gets a copy of the built-in ones.
- **Translate**: `[translate] target = "en"`, `targets = ["en", "de", "fa"]`. `{target}` in
  any prompt becomes `target` (an ISO 639-1 code in the built-in prompt); the popup's `T`
  cycles through `targets` and saves the choice, the CLI takes `--target`.
- **Modifiers** are `[[modifiers]]` tables `{ id, name, key, suffix }` (built in: shorter `-`,
  longer `+`, warmer `w`, formal `f`, British `b`, American `a`). The suffixes of the ones
  switched on (and the context line) are added to the prompt just before the text. Your
  `[[modifiers]]` replace the list; their keys must not clash with action keys (an error).
  A built-in modifier whose key one of your actions uses just loses its key.
- **History**: `general.history = true` (default) keeps runs for `undo` and the recent
  instructions, see [CLI](cli.md#cli) (and [security.md](security.md#history) for what it stores).
- **Output shape**: wrapping quotes or `<text>` tags the model adds are removed (quotes are
  kept when your text was quoted), and the result gets your text's leading indentation and
  trailing newline back.
- **Limits**: `general.max_input_chars` (default 40000) refuses longer text before
  anything is sent; `provider.max_tokens` defaults to 16384.
- **Appearance**: `[appearance]` holds the popup's look: `theme = "system"` (follows the
  desktop) or `"light"` / `"dark"`, `zoom = 1.0` (0.8–1.4, also `Ctrl+=` / `Ctrl+-` /
  `Ctrl+0` in the popup), `font_size = 14` (px), `font = ""` (a font family; empty = the
  system font), `accent = ""` (`"#rrggbb"`; empty = the theme's), and the size the popup
  opens at, `window_width = 720` / `window_height = 520` (at least 520 × 360; a window
  resized by hand keeps its size until these change).
- **Hotkey**: `general.hotkey` is empty (off) by default. It only works on X11; on Wayland
  bind `mirzabenevis --popup` in your compositor or desktop keyboard settings instead.
