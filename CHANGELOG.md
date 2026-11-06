# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## 0.1.0 — Unreleased

First release.

### Added

- Select text in any app, press a key, and get it fixed, rewritten or translated, pasted
  back over the selection (Wayland and X11).
- Finds the AI you already have without configuration: API keys (Anthropic, DeepSeek,
  OpenAI, Gemini, OpenRouter, ...), agent CLIs (Claude Code, Codex, Gemini CLI, `llm`),
  local servers (Ollama, LM Studio, llama.cpp, vLLM, Jan) or a custom endpoint, with
  fallback to the next one when a backend fails.
- Popup with numbered actions, streaming results, an inline word diff, "explain what
  changed", modifiers (shorter, longer, warmer, formal, British, American) and a backend
  picker.
- Your own instructions: type one, refine the result, save it as a new action with ★.
- Chat when nothing is selected; summaries, key points, tables and explanations rendered
  as Markdown.
- History of runs and undo (pastes the original back).
- `mirzabenevis-cli`: run any action from a key binding or a pipe, list and check
  backends, manage the config.
- `mirzabenevis-cli doctor`: checks the grab and paste tools and backends, and prints the
  key-binding and autostart lines for your desktop.
- Packages: a Nix flake (app, CLI, Home Manager module), and `.deb` / AppImage builds on
  GitHub Releases.
