# Changelog

All notable changes to TopClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Docker desktop automation**: dev stage now includes Xvfb (virtual X11 display), xdotool, wmctrl, scrot, xdg-utils, Chromium, and font support so `computer_use` works inside containers. A `dev-entrypoint.sh` script auto-starts Xvfb before the TopClaw gateway.

### Changed
- **CLI simplification**: merged `topclaw peripheral` into `topclaw hardware`; removed obsolete `auth setup-token` (use `auth add-key`).
- **Streaming think-tag stripping**: `<think>` blocks from reasoning models (MiniMax M2.7, Qwen3, GLM-4) are now stripped from both streaming drafts and final channel responses.
- **Crate safety**: added `#![forbid(unsafe_code)]` to `src/lib.rs` and `src/main.rs`.

### Removed
- Deprecated CLI aliases (`topclaw onboard`, `topclaw init`, `chat`, `run`, `info`, `check`, `channels`, `skill`, `self-improve`).
- Deprecated Unix installer wrappers `topclaw_install.sh` and `scripts/install.sh`.
- Deprecated config alias `runtime.reasoning_level` and its environment aliases.
- Legacy `enc:` XOR-encrypted secret support and migration helpers.
- All phantom channel code (Slack, Mattermost, Matrix, WhatsApp, Signal, Email, IRC, Lark, Feishu, DingTalk, QQ, Nextcloud Talk, Linq, iMessage, Nostr). Only Telegram, Discord, bridge, and webhook channels remain.
- Phantom channel config secret encryption/decryption blocks.
- Stale integration API gateway routes.

### Fixed
- **Desktop automation tool selection for non-CLI channels**: `computer_use` now appears before `web_fetch`/`browser_open` in tool descriptions so the LLM picks it for "open Chrome" / "open app" requests; tool description enhanced with explicit `app_launch` + URL routing hints and examples; Desktop Automation system prompt section added with headless environment detection (`$DISPLAY` not set on Linux); `browser_open` registration no longer gated on `has_shell_access`; `browser_open` removed from `non_cli_excluded_tools` in dev config template.
- **Gemini thinking model support** — provider skips internal reasoning parts (`thought: true`) and signature parts, extracting only the final answer text.
- Updated default gateway port to `42617`.
- **Onboarding channel menu dispatch** uses enum-backed selector instead of hard-coded numeric match arms.
- **OpenAI native tool spec parsing** uses owned serializable/deserializable structs, fixing compile-time type mismatch.
- **Unsafe Debt Audit CI** now passes (crate roots declare `#![forbid(unsafe_code)]`).

## [0.1.0] - 2026-02-13

### Added
- **Core Architecture**: Trait-based pluggable system for Provider, Channel, Observer, RuntimeAdapter, Tool
- **Provider**: OpenRouter implementation (access Claude, GPT-4, Llama, Gemini via single API)
- **Channels**: CLI channel with interactive and single-message modes
- **Observability**: NoopObserver (zero overhead), LogObserver (tracing), MultiObserver (fan-out)
- **Security**: Workspace sandboxing, command allowlisting, path traversal blocking, autonomy levels (ReadOnly/Supervised/Full), rate limiting
- **Tools**: Shell (sandboxed), FileRead (path-checked), FileWrite (path-checked)
- **Memory (Brain)**: SQLite persistent backend (searchable, survives restarts), Markdown backend (plain files, human-readable)
- **Heartbeat Engine**: Periodic task execution from HEARTBEAT.md
- **Runtime**: Native adapter for Mac/Linux/Raspberry Pi
- **Config**: TOML-based configuration with sensible defaults
- **Onboarding**: Interactive CLI wizard with workspace scaffolding
- **CLI Commands**: agent, gateway, status, cron, channel, tools, onboard
- **CI/CD**: GitHub Actions with cross-platform builds (Linux, macOS Intel/ARM, Windows)
- **Tests**: 159 inline tests covering all modules and edge cases
- **Binary**: 3.1MB optimized release build (includes bundled SQLite)

### Security
- Path traversal attack prevention
- Command injection blocking
- Workspace escape prevention
- Forbidden system path protection (`/etc`, `/root`, `~/.ssh`)

[0.1.0]: https://github.com/topway-ai/topclaw/releases/tag/v0.1.0
