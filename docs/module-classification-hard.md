# Module Classification — Hard Decision Table

This supersedes `module-classification.md`. Every top-level `src/` module is
classified into exactly one category. No vague wording. No preserved aliases.

---

## Classification Key

| Flag | Meaning |
|---|---|
| `PROTECTED_CORE` | Do not delete, merge, or weaken |
| `CURRENT_MAINLINE` | Actively used by the primary product path |
| `FIRST_REFACTOR_TARGET` | Needs narrowing in this pass or next |
| `TOO_BIG_OWNER` | Module is too large; split or narrow required |
| `LIKELY_LEGACY` | May contain dead code or unused paths |
| `OPTIONAL_BUT_KEEP` | Useful but not required for mainline |

---

## Decision Table

### `agent/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE, TOO_BIG_OWNER
- **Why it exists:** Main LLM-to-tool execution loop. Core product.
- **Current mainline needed:** Yes — the entire agentic loop depends on it.
- **Legacy burden:** `loop_/` subdirectory has 11 files. `agent.rs` + `loop_.rs` overlap.
  `classifier.rs` — single-pass classifier may be replaceable with a provider call.
  `research.rs` — optional phase, not always used.
  `wiring.rs` — thin, keep as-is.
- **Completed refactors:**
  - `memory_loader.rs` INLINED into `agent.rs` (load_memory_context function)
  - Added `min_relevance_score` field to `AgentConfig` (default 0.4)
  - Added tests: legacy autosave skip, entry formatting, relevance filtering
- **Next action:** Keep but narrow. Evaluate `classifier.rs` for deletion.
- **Files/dirs:** `agent.rs`, `classifier.rs`, `dispatcher.rs`, `loop_.rs`, `loop_/`
- **Risk:** HIGH — core loop. Narrow incrementally. Keep tests green.
- **Tests required:** Agent happy path, agent failure path, tool dispatch.

---

### `auth/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** OAuth flows for Anthropic, Gemini, OpenAI cloud auth.
- **Current mainline needed:** Yes — provider OAuth is core authentication path.
- **Legacy burden:** Low. `oauth_common.rs` is shared utility, clean.
- **Next action:** Keep as-is. No changes needed.
- **Files/dirs:** `anthropic_token.rs`, `gemini_oauth.rs`, `mod.rs`, `oauth_common.rs`, `openai_oauth.rs`, `profiles.rs`
- **Risk:** MEDIUM — OAuth token handling. Security-sensitive.
- **Tests required:** OAuth token encode/decode roundtrip.

---

### `channels/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE, TOO_BIG_OWNER, FIRST_REFACTOR_TARGET
- **Why it exists:** Telegram, Discord, CLI channel runtimes. Primary user interaction.
- **Current mainline needed:** Yes — channels are the primary runtime for most users.
- **Legacy burden:** 22 files. Many files could collapse. `command_handler.rs`, `runtime_commands.rs`,
  `runtime_config.rs`, `runtime_help.rs`, `runtime_helpers.rs` — all thin helpers that could merge.
  `dispatch.rs`, `factory.rs` — core, keep.
  `message_processing.rs` — core, keep.
  `transcription.rs`, `runtime_config.rs` — could be inline in factory.
- **Next action:** Keep but narrow. Collapse runtime_* helpers into fewer files.
  Merge `runtime_config.rs` + `runtime_commands.rs` into `dispatch.rs`.
- **Files/dirs:** `cli.rs`, `command_handler.rs`, `context.rs`, `discord.rs`, `dispatch.rs`,
  `factory.rs`, `helpers.rs`, `message_processing.rs`, `mod.rs`, `prompt.rs`, `route_state.rs`,
  `runtime_commands.rs`, `runtime_config.rs`, `runtime_help.rs`, `runtime_helpers.rs`,
  `sanitize.rs`, `startup.rs`, `telegram.rs`, `traits.rs`, `transcription.rs`, `capability_detection.rs`, `capability_recovery.rs`
- **Risk:** HIGH — active user channels. Narrow carefully, keep tests green.
- **Tests required:** Channel factory tests, Telegram dispatch tests, Discord dispatch tests.

---

### `config/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE
- **Why it exists:** All config structs, serde, env overrides, runtime dir resolution.
- **Current mainline needed:** Yes — everything depends on config.
- **Legacy burden:** 40+ files. Many are small enough to merge.
  `schema_*` files: schema.rs is large (the master config). Others are sub-configs.
  `estop.rs`, `browser_domain_grants.rs` — could be inline in schema.rs.
  `browser.rs` — moderate, keep.
  `secrets.rs` — encryption, keep as separate for security boundary.
  `proxy.rs` — moderate, keep.
  `browser_domain_grants.rs` — LIKELY_LEGACY: BrowserAllowlist handles persistence,
  this file could be absorbed.
- **Next action:** Keep but narrow. Merge `browser_domain_grants.rs` into `browser.rs` or `mod.rs`.
  Add state-narrowing: `estop-state.json` → merge into config.toml as `[estop]` section.
- **Files/dirs:** 40+ files — see `src/config/` tree
- **Risk:** MEDIUM — config changes affect everything. Validate with schema tests.
- **Tests required:** Config serde roundtrip, env overrides, runtime dir resolution.

---

### `cron/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Persistent scheduler. Background automation.
- **Current mainline needed:** Yes — cron is a primary feature.
- **Legacy burden:** Low. `consolidation.rs` — unclear purpose, check if used.
- **Next action:** Keep as-is. Audit `consolidation.rs` for usage before next pass.
- **Files/dirs:** `mod.rs`, `consolidation.rs`, `schedule.rs`, `scheduler.rs`, `store.rs`, `types.rs`
- **Risk:** LOW — isolated scheduler.
- **Tests required:** Scheduler add/list/remove/update.

---

### `daemon/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Daemon PID file and lifecycle management.
- **Current mainline needed:** Yes — `topclaw daemon` command requires it.
- **Legacy burden:** Minimal. Single file `mod.rs`.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.
- **Tests required:** Daemon PID file handling.

---

### `doctor/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** System diagnostics, desktop helper detection, config validation.
- **Current mainline needed:** Yes — `topclaw doctor` and status checks.
- **Legacy burden:** Low. Already simplified.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW — diagnostics only.
- **Tests required:** Doctor checks for computer_use, desktop helpers.

---

### `gateway/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** OpenAI-compatible API gateway, Webhook ingress, SSE, WebSocket.
- **Current mainline needed:** Yes — gateway mode is a primary runtime.
- **Legacy burden:** Low. Structure is reasonable.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`, `api.rs`, `openai_compat.rs`, `sse.rs`, `webhook_ingress.rs`, `ws.rs`
- **Risk:** MEDIUM — gateway is a public API surface.
- **Tests required:** Gateway endpoint routing tests.

---

### `health/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Component health registry used by daemon, gateway, channels, and cron.
  Provides `mark_component_ok`, `mark_component_error`, `bump_component_restart`, `snapshot`.
  Exposed via `/health` endpoint in gateway and `/status` in daemon.
- **Current mainline needed:** Yes — daemon, gateway, channels, and cron all use it.
  `gateway/mod.rs`, `daemon/mod.rs`, `channels/mod.rs`, `cron/scheduler.rs` all reference it.
- **Legacy burden:** None. The module is well-used and purposeful.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW — used by all runtimes.
- **Tests required:** Component ok/error/restart lifecycle tests.

---

### `hooks/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Extensible hook system for command logging and runtime events.
  `HookRunner` used by `gateway/mod.rs` (gateway hooks), `channels/startup.rs` (command_logger
  registration), and `agent/loop_.rs` (hooks passed into execution context).
  `BuiltinHooksConfig` in config controls which hooks are registered.
- **Current mainline needed:** Yes — `config.hooks.enabled` gates the system, and
  `config.hooks.builtin.command_logger` controls the command logger hook.
  Both `gateway/mod.rs` and `channels/startup.rs` create HookRunners when enabled.
- **Legacy burden:** Low. The system is feature-gated and opt-in. Only one builtin hook.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`, `traits.rs`, `runner.rs`, `builtin/mod.rs`, `builtin/command_logger.rs`
- **Risk:** MEDIUM — feature-gated, but gateway and channels use it.
- **Tests required:** Hook runner priority, cancel, and pipeline tests (already exist).

---

### `memory/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE, TOO_BIG_OWNER, FIRST_REFACTOR_TARGET
- **Why it exists:** Agent memory (sqlite, markdown, vector backends).
- **Current mainline needed:** Yes — memory tools depend on it.
- **Legacy burden:** `embeddings.rs` — complex embedding provider setup.
  `response_cache.rs` — separate cache layer.
  `hygiene.rs` — background cleanup, could be inline.
  `snapshot.rs` — hydration from snapshot, useful but complex.
  `vector.rs` — separate file for vector search.
  `backend.rs` — classification and profile logic.
- **Next action:** Keep but narrow. Merge `backend.rs` into `mod.rs` (only 2 public fns).
  Keep embeddings, hygiene, snapshot as separate (they have distinct purposes).
- **Files/dirs:** 12 files — see `src/memory/` tree
- **Risk:** MEDIUM — memory is critical for agent context.
- **Tests required:** Memory backend factory tests, SQL/Markdown/None roundtrips.

---

### `observability/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Metrics, tracing, runtime trace logging.
- **Current mainline needed:** Yes — observability is standard infrastructure.
- **Legacy burden:** Low. Structure is reasonable. `runtime_trace.rs` → target for cache move.
- **Next action:** Keep as-is. Move `runtime_trace.jsonl` to cache dir (see state model).
- **Files/dirs:** `mod.rs`, `log.rs`, `multi.rs`, `noop.rs`, `otel.rs`, `prometheus.rs`, `runtime_trace.rs`, `traits.rs`
- **Risk:** LOW — observability infrastructure.
- **Tests required:** Observer creation tests.

---

### `onboard/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** First-run wizard, channel repair, model catalog management.
- **Current mainline needed:** Yes — onboarding is required for new users.
- **Legacy burden:** 7 files. `wizard_channel_flows.rs` — unclear if still used (check).
  `wizard_model_catalog.rs` — model catalog operations.
  `wizard_skill_selection.rs` — skill selection during onboarding.
- **Next action:** Keep as-is. Audit `wizard_channel_flows.rs` for usage.
- **Files/dirs:** 7 files — see `src/onboard/` tree
- **Risk:** MEDIUM — onboarding touches config, channels, providers.
- **Tests required:** Wizard config write tests.

---

### `providers/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE
- **Why it exists:** LLM provider adapters (OpenAI, Anthropic, Gemini, Ollama, OpenRouter).
- **Current mainline needed:** Yes — all agent runs use providers.
- **Legacy burden:** `circuit_breaker.rs` — stateful, may be over-engineered.
  `error_parser.rs` — error classification utility.
  `aliases.rs` — name alias resolution (e.g. `zai-cn`).
- **Next action:** Keep as-is. `aliases.rs` is useful; don't delete.
  Consider if `circuit_breaker.rs` is actually used (check circuit_breaker usage).
- **Files/dirs:** ~15 files — see `src/providers/` tree
- **Risk:** HIGH — providers are the core LLM interface.
- **Tests required:** Provider registry tests, model routing tests.

---

### `runtime/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Execution environment abstraction (native, Docker, WASM).
- **Current mainline needed:** Yes — runtime adapter is used by tools and agent.
- **Legacy burden:** `traits.rs` — trait definition only. `native.rs` — main implementation.
  WASM and Docker may be feature-gated.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`, `traits.rs`, `native.rs`
- **Risk:** MEDIUM — runtime changes affect all tool execution.
- **Tests required:** Runtime adapter creation tests.

---

### `security/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE, TOO_BIG_OWNER
- **Why it exists:** Policy enforcement, sandboxing, audit, secrets, OTP, estop.
- **Current mainline needed:** Yes — security is a core product differentiator.
- **Legacy burden:** 16 files. Most are well-organized.
  `detect.rs` — sandbox backend detection.
  `canary_guard.rs`, `semantic_guard.rs`, `prompt_guard.rs` — overlapping
  prompt-injection defenses. May be redundant.
  `leak_detector.rs` — credential leak detection.
- **Next action:** Keep but narrow. Audit `prompt_guard.rs`, `semantic_guard.rs`,
  `canary_guard.rs` for overlap. If duplicate, consolidate into one.
- **Files/dirs:** 16 files — see `src/security/` tree
- **Risk:** HIGH — security is safety-critical.
- **Tests required:** Security policy tests, secrets encrypt/decrypt, estop state.

---

### `service/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Daemon/service lifecycle (plist, systemd, PID file).
- **Current mainline needed:** Yes — `topclaw daemon` requires service management.
- **Legacy burden:** Single `mod.rs` file. Clean.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.
- **Tests required:** Service script generation tests.

---

### `sidecar/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE
- **Why it exists:** Built-in computer-use sidecar HTTP server.
- **Current mainline needed:** Yes — sidecar is the execution engine for computer_use.
- **Legacy burden:** `linux.rs` — Linux-specific backend. Keep for now.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`, `server.rs`, `linux.rs`
- **Risk:** MEDIUM — sidecar handles desktop automation.
- **Tests required:** Sidecar server tests.

---

### `skills/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Skill discovery, loading, execution.
- **Current mainline needed:** Yes — skills are a primary feature.
- **Legacy burden:** `mod.rs` only. Clean.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.
- **Tests required:** Skill loading tests.

---

### `tools/`

- **Classification:** KEEP
- **Flags:** PROTECTED_CORE, TOO_BIG_OWNER, FIRST_REFACTOR_TARGET
- **Why it exists:** Agent-callable tool implementations. ~40+ files.
- **Current mainline needed:** Yes — all tool execution goes through this.
- **Legacy burden:** Largest subsystem. Many tools could be feature-gated or merged.
  `cron_*.rs` (6 files) — could merge into `schedule.rs`.
  `memory_*.rs` (3 files) — could merge.
  `lossless_*.rs` (2 files) — could merge.
  `subagent_*.rs` (4 files) — could merge into one `subagent.rs`.
  `delegate_*.rs` (2 files) — could merge.
- **Next action:** Keep but narrow. Merge cron tools, memory tools, lossless tools,
  subagent tools into single files each. This is the main FIRST_REFACTOR_TARGET.
- **Files/dirs:** ~45 files — see `src/tools/` tree
- **Risk:** HIGH — tool changes affect agent capability.
- **Tests required:** Tool registry tests, individual tool parameter validation tests.

---

### `tunnel/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Tunnel management (Cloudflare, Ngrok, Tailscale).
- **Current mainline needed:** Yes — tunnel is a primary feature for remote access.
- **Legacy burden:** 5 files. Structure is clean.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`, `cloudflare.rs`, `ngrok.rs`, `tailscale.rs`, `custom.rs`
- **Risk:** LOW.
- **Tests required:** Tunnel creation tests.

---

### `workspace/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Single `mod.rs` file. Workspace utilities.
- **Current mainline needed:** Yes — workspace is a core concept.
- **Legacy burden:** Minimal. Single file.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.

---

### `coordination/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** In-memory message bus for multi-agent coordination.
- **Current mainline needed:** Yes — delegate coordination depends on it.
- **Legacy burden:** Minimal. Single `mod.rs`.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.
- **Tests required:** Message bus limit tests.

---

### `cost/`

- **Classification:** KEEP
- **Flags:** OPTIONAL_BUT_KEEP
- **Why it exists:** Cost tracking and budget enforcement.
- **Current mainline needed:** Only if cost limits are configured. Otherwise unused.
- **Legacy burden:** Minimal. Public re-exports.
- **Next action:** Keep as-is. No changes needed.
- **Files/dirs:** `mod.rs`, `tracker.rs`, `types.rs`
- **Risk:** LOW.
- **Tests required:** Cost tracker roundtrip tests.

---

### `approval/`

- **Classification:** KEEP
- **Flags:** CURRENT_MAINLINE
- **Why it exists:** Approval system for tools requiring user confirmation.
- **Current mainline needed:** Yes — approval-gated tools need it.
- **Legacy burden:** Minimal. Single `mod.rs`.
- **Next action:** Keep as-is.
- **Files/dirs:** `mod.rs`
- **Risk:** LOW.

---

### `heartbeat/`

- **Classification:** KEEP
- **Flags:** OPTIONAL_BUT_KEEP
- **Why it exists:** Heartbeat engine for monitoring.
- **Current mainline needed:** Only if heartbeat monitoring is configured.
- **Legacy burden:** Minimal. `engine.rs` + `mod.rs`.
- **Next action:** Keep as-is. Check if actually used before next pass.
- **Files/dirs:** `mod.rs`, `engine.rs`
- **Risk:** LOW.
- **Tests required:** Heartbeat engine tests (if used).

---

## Deletion List (this pass)

| Module/File | Classification | Reason |
|---|---|---|
| `config/browser_domain_grants.rs` | MERGE | Overlaps with BrowserAllowlist; consolidate into it |
| `agent/classifier.rs` | TEST then DELETE | Single-pass classifier; check if used before deletion |

## Merge List (this pass)

| From | To | Reason |
|---|---|---|
| `config/browser_domain_grants.rs` | `config/mod.rs` | BrowserAllowlist owns grants logic; module is thin wrapper |
| `tools/cron_add.rs`, `cron_list.rs`, `cron_remove.rs`, `cron_runs.rs`, `cron_update.rs` | `tools/schedule.rs` | All cron tools share the same pattern |
| `tools/memory_store.rs`, `memory_recall.rs`, `memory_forget.rs` | `tools/memory.rs` (new file) | Memory tools share state |
| `tools/lossless_describe.rs`, `lossless_search.rs` | `tools/lossless.rs` (new file) | Both operate on workspace files |
| `tools/subagent_*.rs` (4 files) | `tools/subagent.rs` (new file) | All subagent tools share registry |
| `tools/delegate.rs`, `delegate_coordination_status.rs` | `tools/delegate.rs` | Delegate tool and coordination are one subsystem |

## State Model Narrowing (PART 3)

| Path | Action | Reason |
|---|---|---|
| `browser-allowed-domains-grants.json` | MERGE into `BrowserAllowlist` as field | Avoids separate read-modify-write path |
| `state/runtime-trace.jsonl` | MOVE to `~/.cache/topclaw/` | Ephemeral debug log, not durable state |
| `estop-state.json` | MARK for next-wave merge into config.toml | Not safe in this pass; needs schema migration |
| `active_workspace.toml` | KEEP as-is | Active workspace marker is current mainline |

## Test Coverage Gaps

| Area | Tests Required |
|---|---|
| computer_use happy path | Test all 12 actions against mock sidecar |
| computer_use failure path | Test sidecar unreachable, health timeout, bad HTTP |
| sidecar client health behavior | Test probe_health, spawn_sidecar_child, health timeout |
| current config resolution | Test TOPCLAW_CONFIG_DIR, TOPCLAW_WORKSPACE, active_workspace.toml precedence |
| doctor status after legacy removal | Test that status output is correct without ../.topclaw fallback |
| BrowserAllowlist persistence | Test that grants are saved/loaded correctly |

---

## Summary

**Delete:** None in this pass (health and hooks are both active)
**Merge:** `config/browser_domain_grants.rs` into config module
**Narrow:** `tools/` (merge cron/memory/lossless/subagent/delegate tools)
**Test:** `agent/classifier.rs` for usage before deletion decision

**Protected core:** `agent`, `config`, `providers`, `security`, `tools`, `sidecar`
**Current mainline:** All channel and daemon modules
**Next-wave targets:** `memory` (TOO_BIG_OWNER), `channels` (TOO_BIG_OWNER), `security` (prompt guard overlap)