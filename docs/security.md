# Security and privacy

What leaves your machine, where, and what is kept. Back to the [README](../README.md).

## Your text stays with one vendor

`provider.use = "auto"` only falls back to another backend **from the same vendor**.
Your text is never sent to a different company behind your back. Details in
[configuration.md](configuration.md#ai-backends-found-automatically).

The selection is sent inside `<text>…</text>` tags and the system prompt tells the model to
treat it as data, not instructions ([configuration.md](configuration.md#config)).
`general.max_input_chars` refuses longer text before anything is sent.

## Agent CLIs

Agent CLIs run in a private empty directory with tools disabled where the CLI allows it
(Claude Code: all; Codex: read-only sandbox, no shell, no web). Claude Code also runs with
`--setting-sources ""`, so your `~/.claude/CLAUDE.md` and settings are not added to the
prompt (your login still works). Gemini CLI keeps its read-only tools, so `"auto"` never
picks it: use it only by pinning it (`config use gemini-cli` or `--backend gemini-cli`),
and prefer the Gemini API for untrusted text.

## History

Every successful run is kept as **plain text** in `$XDG_DATA_HOME/mirzabenevis/history.jsonl`
(mode 600, not encrypted, ends up in your backups); turn it off with `general.history = false`.
Chats are not written to the history. See [cli.md](cli.md#cli).

## Clipboard

A clipboard marked as a password (`x-kde-passwordManagerHint`, set by KeePassXC and
others) is not saved or restored. Only text is restored after a paste. See
[Grab and paste on Linux](cli.md#grab-and-paste-on-linux).
