# Module Classification — `src/`

This document classifies every top-level module under `src/` into one of four
strict categories. The purpose is to make ownership, review responsibility,
and dependency direction explicit so that future changes can be assessed
against a known boundary.

## Categories

| Category | Abbrev | Meaning | Allowed to import |
|---|---|---|---|
| **Core** | C | Application entry-point, config boot, and top-level orchestration | All |
| **Config** | CF | Data structures that define how TopClaw is configured | `std`, third-party crates only; **never** `tools`, `providers`, `security`, `skills` |
| **Provider** | P | LLM provider adapters (OpenAI, Anthropic, Gemini, etc.) | `config`, `security` (traits only) |
| **Security** | S | Policy enforcement, sandboxing, audit, secrets | `config` (traits only) |
| **Tool** | T | Agent-callable tool implementations | `config`, `security`, `sidecar_client` |
| **Service** | SV | Daemon/service lifecycle (plist, systemd, PID file) | `config` |
| **Observability** | O | Metrics, tracing, runtime trace | `config` (read-only) |
| **Runtime** | R | Execution environment abstraction (native, Docker, WASM) | `config` |
| **Skill** | SK | Skill discovery, loading, and execution | `config`, `security` |
| **Onboarding** | OB | Interactive first-run wizard flows | `config`, `skills`, `tools` (construction only) |
| **State** | ST | Persistent state (workspace markers, auth profiles, secrets) | `config` |

---

## Classification Table

| Module | Category | Notes |
|---|---|---|
| `main.rs` | C | CLI entry-point, argument parsing, command dispatch |
| `main_handlers.rs` | C | Sub-command handler functions called from main |
| `lib.rs` | C | Library root — re-exports the public API surface |
| `util.rs` | C | Small shared helpers (no domain logic) |
| `multimodal.rs` | C | Multimodal config wiring for providers |
| `update.rs` | C | Self-update check and binary replacement |
| `backup.rs` | ST | Config directory backup/restore staging |
| `test_capabilities.rs` | C | Build-time capability probe (test-only) |
| **`config/`** | CF | Config schema, serde, env overrides, runtime dir resolution |
| `config/mod.rs` | CF | Re-exports, `Config::load_or_init`, `apply_env_overrides` |
| `config/schema.rs` | CF | Master `Config` struct and all sub-config structs |
| `config/browser.rs` | CF | `BrowserConfig`, `BrowserComputerUseConfig` |
| `config/schema_secrets.rs` | CF | Secret encryption/decryption at save time |
| `config/schema_runtime_dirs.rs` | CF | Workspace/config directory resolution |
| `config/schema_tests.rs` | CF | Config serde roundtrip and override tests |
| `config/autonomy.rs` | CF | Autonomy level defaults and excluded-tools list |
| `config/proxy.rs` | CF | Proxy configuration and cache |
| `config/browser_domain_grants.rs` | CF | Persistent browser domain approval grants |
| `config/estop.rs` | CF | Emergency stop state file path |
| **`providers/`** | P | LLM provider adapters |
| `providers/mod.rs` | P | Provider registry, `DEFAULT_PROVIDER_NAME` |
| `providers/traits.rs` | P | `Provider` trait definition |
| `providers/openai.rs` | P | OpenAI chat-completions adapter |
| `providers/openai_codex.rs` | P | OpenAI Codex (responses API) adapter |
| `providers/anthropic.rs` | P | Anthropic Claude adapter |
| `providers/gemini.rs` | P | Google Gemini adapter |
| `providers/ollama.rs` | P | Ollama local adapter |
| `providers/openrouter.rs` | P | OpenRouter routing adapter |
| `providers/compatible.rs` | P | Generic OpenAI-compatible adapter |
| `providers/aliases.rs` | P | Provider name alias resolution (e.g. `zai-cn`) |
| `providers/registry.rs` | P | Provider factory lookup |
| `providers/router.rs` | P | Model-route resolution |
| `providers/reliable.rs` | P | Retry/circuit-breaker wrapper |
| `providers/circuit_breaker.rs` | P | Circuit-breaker state machine |
| `providers/health_probe.rs` | P | Provider health-check probe |
| `providers/error_parser.rs` | P | Structured error parsing from provider responses |
| **`security/`** | S | Policy enforcement, sandboxing, audit |
| `security/mod.rs` | S | `SecurityPolicy` main struct |
| `security/traits.rs` | S | Security trait definitions |
| `security/policy.rs` | S | Policy evaluation (path allow, command allow, OTP) |
| `security/audit.rs` | S | Audit event logging |
| `security/bubblewrap.rs` | S | Bubblewrap sandbox integration |
| `security/docker.rs` | S | Docker sandbox integration |
| `security/firejail.rs` | S | Firejail sandbox integration |
| `security/landlock.rs` | S | Linux Landlock LSM integration |
| `security/detect.rs` | S | Anomaly detection heuristics |
| `security/estop.rs` | S | Emergency stop enforcement |
| `security/otp.rs` | S | One-time password gating |
| `security/pairing.rs` | S | Gateway pairing handshake |
| `security/prompt_guard.rs` | S | Prompt injection detection |
| `security/semantic_guard.rs` | S | Semantic policy guard |
| `security/secrets.rs` | S | Encrypted secret store (`~/.topclaw/.secret_key`) |
| `security/canary_guard.rs` | S | Canary token detection |
| `security/domain_matcher.rs` | S | Domain allowlist matching |
| `security/leak_detector.rs` | S | Credential leak detection in output |
| `security/syscall_anomaly.rs` | S | Syscall pattern anomaly detector |
| **`tools/`** | T | Agent-callable tool implementations |
| `tools/mod.rs` | T | Tool registry assembly (`all_tools`, `channel_tools`) |
| `tools/traits.rs` | T | `Tool` and `ToolResult` trait definitions |
| `tools/schema.rs` | T | Tool spec serialization |
| `tools/shell.rs` | T | Shell command execution |
| `tools/process.rs` | T | Process management tool |
| `tools/file_read.rs` | T | File reading tool |
| `tools/file_write.rs` | T | File writing tool |
| `tools/file_edit.rs` | T | File editing tool |
| `tools/apply_patch.rs` | T | Unified patch application |
| `tools/glob_search.rs` | T | Glob pattern file search |
| `tools/content_search.rs` | T | Ripgrep content search |
| `tools/browser.rs` | T | Browser automation (pluggable backends) — `browser-native` feature |
| `tools/browser_open.rs` | T | Simple URL-open tool |
| `tools/screenshot.rs` | T | Screenshot capture tool |
| `tools/computer_use.rs` | T | General-purpose desktop automation — `computer-use-sidecar` feature |
| `tools/computer_use_sidecar_start.rs` | T | Sidecar lifecycle tool — `computer-use-sidecar` feature |
| `tools/sidecar_client.rs` | T | Shared sidecar client (health probe, spawn, URL derivation) — `computer-use-sidecar` feature |
| `tools/memory_store.rs` | T | Memory store tool |
| `tools/memory_recall.rs` | T | Memory recall tool |
| `tools/memory_forget.rs` | T | Memory forget tool |
| `tools/web_fetch.rs` | T | Web content fetcher |
| `tools/web_search_tool.rs` | T | Web search tool |
| `tools/http_request.rs` | T | HTTP request tool |
| `tools/git_operations.rs` | T | Git operations tool |
| `tools/delegate.rs` | T | Delegate tool (multi-agent) |
| `tools/delegate_coordination_status.rs` | T | Coordination bus status tool |
| `tools/subagent_spawn.rs` | T | Sub-agent spawn tool |
| `tools/subagent_list.rs` | T | Sub-agent list tool |
| `tools/subagent_manage.rs` | T | Sub-agent manage tool |
| `tools/subagent_registry.rs` | T | Sub-agent registry |
| `tools/task_plan.rs` | T | Task planning tool |
| `tools/cron_*.rs` | T | Cron add/list/remove/run/update/runs tools |
| `tools/config_patch.rs` | T | Config patch tool |
| `tools/config_grant_browser_domain.rs` | T | Browser domain grant tool |
| `tools/proxy_config.rs` | T | Proxy configuration tool |
| `tools/model_routing_config.rs` | T | Model routing config tool |
| `tools/lossless_describe.rs` | T | Lossless file description |
| `tools/lossless_search.rs` | T | Lossless file search |
| `tools/image_info.rs` | T | Image metadata tool |
| `tools/pdf_read.rs` | T | PDF reading tool — `rag-pdf` feature |
| `tools/composio.rs` | T | Composio integration tool — `tool-composio` feature |
| `tools/discord_history_fetch.rs` | T | Discord history fetch — `tool-discord` feature |
| `tools/url_validation.rs` | T | URL validation helper |
| `tools/path_resolution.rs` | T | Path resolution for file tools |
| `tools/policy_gate.rs` | T | Tool policy gate |
| `tools/cli_discovery.rs` | T | CLI binary discovery |
| `tools/channel_runtime_context.rs` | T | Channel runtime context tool |
| **`service/`** | SV | Daemon/service lifecycle |
| `service/mod.rs` | SV | Service registration, PID file, plist management |
| **`observability/`** | O | Metrics and tracing |
| `observability/mod.rs` | O | Observability dispatch |
| `observability/otel.rs` | O | OpenTelemetry integration |
| `observability/prometheus.rs` | O | Prometheus metrics |
| `observability/multi.rs` | O | Multi-backend combiner |
| `observability/noop.rs` | O | No-op backend |
| `observability/runtime_trace.rs` | O | JSONL runtime trace |
| **`runtime/`** | R | Execution environment abstraction |
| `runtime/mod.rs` | R | Runtime trait and factory |
| `runtime/traits.rs` | R | `RuntimeAdapter` trait |
| `runtime/native.rs` | R | Native runtime implementation |
| **`skills/`** | SK | Skill system |
| `skills/mod.rs` | SK | Skill discovery, loading, and execution |
| **`onboard/`** | OB | First-run wizard |
| `onboard/mod.rs` | OB | Onboarding entry-point |
| `onboard/wizard.rs` | OB | Main wizard flow |
| `onboard/wizard_channels.rs` | OB | Channel configuration wizard |
| `onboard/wizard_provider_setup.rs` | OB | Provider setup wizard |
| `onboard/wizard_model_catalog.rs` | OB | Model catalog wizard |
| `onboard/wizard_skill_selection.rs` | OB | Skill selection wizard |
| **`sidecar/`** | SV | Sidecar server |
| `sidecar/mod.rs` | SV | Sidecar subcommand entry |
| `sidecar/server.rs` | SV | Sidecar HTTP server |
| `sidecar/linux.rs` | SV | Linux desktop automation backend |
| **`tunnel/`** | SV | Tunnel management |
| `tunnel/mod.rs` | SV | Tunnel dispatch |
| `tunnel/cloudflare.rs` | SV | Cloudflare tunnel |
| `tunnel/ngrok.rs` | SV | Ngrok tunnel |
| `tunnel/tailscale.rs` | SV | Tailscale tunnel |
| `tunnel/custom.rs` | SV | Custom tunnel |

## Dependency Direction Rules

1. **CF → nothing domain-specific.** Config structs must not import `tools`,
   `providers`, `security`, or `skills`. They may import `std` and
   third-party crates (serde, schemars, reqwest for URL parsing).

2. **T → CF + S.** Tools import config structs for construction and security
   policy for enforcement. Tools never import providers directly.

3. **P → CF + S(traits).** Providers import config for API key/URL resolution
   and security traits for policy checks. Providers never import tools.

4. **S → CF(traits).** Security imports config for policy data. Never imports
   tools or providers.

5. **SK → CF + S.** Skills import config for skill directories and security
   for policy checks.

6. **OB → CF + SK + T(construction).** Onboarding may construct tools and
   skills but must not call tool execution methods.

7. **C → all.** Core may import anything, but should prefer thin orchestration
   over domain logic.

## Feature-Gated Modules

Several modules are conditionally compiled:

| Feature flag | Modules gated |
|---|---|
| `browser-native` | `tools::browser` |
| `computer-use-sidecar` | `tools::computer_use`, `tools::computer_use_sidecar_start`, `tools::sidecar_client` |
| `rag-pdf` | `tools::pdf_read` |
| `tool-composio` | `tools::composio` |
| `tool-discord` | `tools::discord_history_fetch` |
| `otel` | `observability::otel` |
