# TopClaw Commands Reference

This reference is derived from the current CLI surface (`topclaw --help`).

Last verified: **March 7, 2026**.

## Top-Level Commands

| Command | Purpose |
|---|---|
| `onboard` | Initialize workspace/config quickly or interactively |
| `agent` | Run interactive chat or single-message mode |
| `gateway` | Start webhook and WhatsApp HTTP gateway |
| `daemon` | Start supervised runtime (gateway + channels + optional heartbeat/scheduler) |
| `service` | Manage user-level OS service lifecycle |
| `doctor` | Run diagnostics and freshness checks |
| `status` | Print current configuration and system summary |
| `update` | Check for or install the latest TopClaw release |
| `estop` | Engage/resume emergency stop levels and inspect estop state |
| `cron` | Manage scheduled tasks |
| `models` | Refresh provider model catalogs |
| `providers` | List provider IDs, aliases, and active provider |
| `channel` | Manage channels and channel health checks |
| `integrations` | Inspect integration details |
| `skills` | List/install/remove skills |
| `migrate` | Import from external runtimes (currently OpenClaw) |
| `config` | Export machine-readable config schema |
| `completions` | Generate shell completion scripts to stdout |
| `hardware` | Discover and introspect USB hardware |
| `peripheral` | Configure and flash peripherals |

Common aliases:

- `topclaw init` -> `topclaw onboard`
- `topclaw chat` -> `topclaw agent`
- `topclaw run` -> `topclaw daemon`
- `topclaw info` -> `topclaw status`
- `topclaw channels` -> `topclaw channel`
- `topclaw skill` -> `topclaw skills`

## Command Groups

### `onboard`

- `topclaw onboard`
- `topclaw onboard --interactive`
- `topclaw onboard --channels-only`
- `topclaw onboard --force`
- `topclaw onboard --api-key <KEY> --provider <ID> --memory <sqlite|lucid|markdown|none>`
- `topclaw onboard --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none>`
- `topclaw onboard --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none> --force`

`onboard` safety behavior:

- If `config.toml` already exists and you run `--interactive`, onboarding now offers two modes:
  - Full onboarding (overwrite `config.toml`)
  - Provider-only update (update provider/model/API key while preserving existing channels, tunnel, memory, hooks, and other settings)
- In non-interactive environments, existing `config.toml` causes a safe refusal unless `--force` is passed.
- Use `topclaw onboard --channels-only` when you only need to rotate channel tokens/allowlists.

### `agent`

- `topclaw agent`
- `topclaw agent -m "Hello"`
- `topclaw agent --provider <ID> --model <MODEL> --temperature <0.0-2.0>`
- `topclaw agent --peripheral <board:path>`

Tip:

- In interactive chat, you can ask for route changes in natural language (for example “conversation uses kimi, coding uses gpt-5.3-codex”); the assistant can persist this via tool `model_routing_config`.

### `gateway` / `daemon`

- `topclaw gateway [--host <HOST>] [--port <PORT>] [--new-pairing]`
- `topclaw daemon [--host <HOST>] [--port <PORT>]`

`--new-pairing` clears all stored paired tokens and forces generation of a fresh pairing code on gateway startup.

### `estop`

- `topclaw estop` (engage `kill-all`)
- `topclaw estop --level network-kill`
- `topclaw estop --level domain-block --domain "*.chase.com" [--domain "*.paypal.com"]`
- `topclaw estop --level tool-freeze --tool shell [--tool browser]`
- `topclaw estop status`
- `topclaw estop resume`
- `topclaw estop resume --network`
- `topclaw estop resume --domain "*.chase.com"`
- `topclaw estop resume --tool shell`
- `topclaw estop resume --otp <123456>`

Notes:

- `estop` commands require `[security.estop].enabled = true`.
- When `[security.estop].require_otp_to_resume = true`, `resume` requires OTP validation.
- OTP prompt appears automatically if `--otp` is omitted.

### `service`

- `topclaw service install`
- `topclaw service start`
- `topclaw service stop`
- `topclaw service restart`
- `topclaw service status`
- `topclaw service uninstall`

### `update`

- `topclaw update`
- `topclaw update --check`
- `topclaw update --force`

Notes:

- `topclaw update` downloads the latest official GitHub release for the current platform and replaces the current binary.
- `--check` only checks whether a newer version is available.
- `--force` reinstalls the latest version even if the current version already matches.
- If the binary location is not writable, TopClaw now prints a recovery path instead of failing silently. On Linux, the recommended fallback is the official release installer:

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/install-release.sh | bash
```

### `cron`

- `topclaw cron list`
- `topclaw cron add <expr> [--tz <IANA_TZ>] <command>`
- `topclaw cron add-at <rfc3339_timestamp> <command>`
- `topclaw cron add-every <every_ms> <command>`
- `topclaw cron once <delay> <command>`
- `topclaw cron remove <id>`
- `topclaw cron pause <id>`
- `topclaw cron resume <id>`

Notes:

- Mutating schedule/cron actions require `cron.enabled = true`.
- Shell command payloads for schedule creation (`create` / `add` / `once`) are validated by security command policy before job persistence.

### `models`

- `topclaw models refresh`
- `topclaw models refresh --provider <ID>`
- `topclaw models refresh --force`

`models refresh` currently supports live catalog refresh for provider IDs: `openrouter`, `openai`, `anthropic`, `groq`, `mistral`, `deepseek`, `xai`, `together-ai`, `gemini`, `ollama`, `llamacpp`, `sglang`, `vllm`, `astrai`, `venice`, `fireworks`, `cohere`, `moonshot`, `glm`, `zai`, `qwen`, and `nvidia`.

#### Live model availability test

```bash
./dev/test_models.sh              # test all Gemini models + profile rotation
./dev/test_models.sh models       # test model availability only
./dev/test_models.sh profiles     # test profile rotation only
```

Runs a Rust integration test (`tests/gemini_model_availability.rs`) that verifies each model against the OAuth endpoint (cloudcode-pa). Requires valid Gemini OAuth credentials in `auth-profiles.json`.

### `doctor`

- `topclaw doctor`
- `topclaw doctor models [--provider <ID>] [--use-cache]`
- `topclaw doctor traces [--limit <N>] [--event <TYPE>] [--contains <TEXT>]`
- `topclaw doctor traces --id <TRACE_ID>`

`topclaw doctor` now ends with concrete next-step commands when it detects actionable setup issues, such as missing provider configuration, missing auth, missing channels, or a missing workspace directory.

### `status`

- `topclaw status`

`topclaw status` prints the current config/runtime summary and now also shows next-step commands for important gaps that still need attention, using the same suggestion logic as `topclaw doctor`.

Provider connectivity matrix CI/local helper:

- `python3 scripts/ci/provider_connectivity_matrix.py --binary target/release-fast/topclaw --contract .github/connectivity/probe-contract.json`

`doctor traces` reads runtime tool/model diagnostics from `observability.runtime_trace_path`.

### `channel`

- `topclaw channel list`
- `topclaw channel start`
- `topclaw channel doctor`
- `topclaw channel bind-telegram <IDENTITY>`
- `topclaw channel add <type> <json>`
- `topclaw channel remove <name>`

Runtime in-chat commands while channel server is running:

- Telegram/Discord sender-session routing:
  - `/models`
  - `/models <provider>`
  - `/model`
  - `/model <model-id>`
  - `/new`
- Supervised tool approvals (all non-CLI channels):
  - `/approve-request <tool-name>` (create pending approval request)
  - `/approve-confirm <request-id>` (confirm pending request; same sender + same chat/channel only)
  - `/approve-pending` (list pending requests in current sender+chat/channel scope)
  - `/approve <tool-name>` (direct one-step grant + persist to `autonomy.auto_approve`, compatibility path)
  - `/unapprove <tool-name>` (revoke + remove from `autonomy.auto_approve`)
  - `/approvals` (show runtime + persisted approval state)
  - Natural-language approval behavior is controlled by `[autonomy].non_cli_natural_language_approval_mode`:
    - `direct` (default): `授权工具 shell` / `approve tool shell` immediately grants
    - `request_confirm`: natural-language approval creates pending request, then confirm with request ID
    - `disabled`: natural-language approval commands are ignored (slash commands only)
  - Optional per-channel override: `[autonomy].non_cli_natural_language_approval_mode_by_channel`

Approval safety behavior:

- Runtime approval commands are parsed and executed **before** LLM inference in the channel loop.
- Pending requests are sender+chat/channel scoped and expire automatically.
- Confirmation requires the same sender in the same chat/channel that created the request.
- Once approved and persisted, the tool remains approved across restarts until revoked.
- Optional policy gate: `[autonomy].non_cli_approval_approvers` can restrict who may execute approval-management commands.

Startup behavior for multiple channels:
- `topclaw channel start` starts all configured channels in one process.
- If one channel fails initialization, other channels continue to start.
- If all configured channels fail initialization, startup exits with an error.

Channel runtime also watches `config.toml` and hot-applies updates to:
- `default_provider`
- `default_model`
- `default_temperature`
- `api_key` / `api_url` (for the default provider)
- `reliability.*` provider retry settings

`add/remove` currently route you back to managed setup/manual config paths (not full declarative mutators yet).

### `integrations`

- `topclaw integrations info <name>`

### `skills`

- `topclaw skills list`
- `topclaw skills vet <source_or_name> [--json] [--sandbox docker]`
- `topclaw skills audit <source_or_name>`
- `topclaw skills install <source>`
- `topclaw skills remove <name>`

`<source>` accepts:

| Format | Example | Notes |
|---|---|---|
| **Preloaded alias** | `find-skills` | Resolved via `<workspace>/skills/.download-policy.toml` aliases |
| **skills.sh URL** | `https://skills.sh/vercel-labs/skills/find-skills` | Parses `owner/repo/skill`, clones source repo, installs the selected skill subdirectory |
| **Git remotes** | `https://github.com/…`, `git@host:owner/repo.git` | Cloned with `git clone --depth 1` |
| **Local filesystem paths** | `./my-skill` or `/abs/path/skill` | Directory copied and audited |

**Domain trust gate (URL installs):**
- First time a URL-based install hits an unseen domain, TopClaw asks whether you trust that domain.
- Trust decisions are persisted in `<workspace>/skills/.download-policy.toml`.
- Trusted domains allow future downloads on the same domain/subdomains; blocked domains are denied automatically.
- Built-in defaults are transparent: preloaded bundles ship in repository `/skills/` and are copied to `<workspace>/skills/` on initialization.
- To pre-configure behavior, edit:
  - `aliases` (custom source shortcuts)
  - `trusted_domains`
  - `blocked_domains`

`skills vet`, `skills audit`, and `skills install` now emit a structured review with:
- `files_scanned`
- `overall risk`: `low`, `medium`, `high`, or `critical`
- per-finding risk/category/message entries
- final verdict: `installable` or `blocked`

`skills vet --sandbox docker` adds an isolated read-only Docker probe with networking disabled. It does not execute the skill itself; it verifies that the skill package can be inspected in a container without write access or outbound network.

`skills install` only accepts skills whose overall audit result is `low`. Any `medium`, `high`, or `critical` finding blocks installation by default.

The audit blocks or escalates findings for:
- symlinks inside the skill package
- script-like files (`.sh`, `.bash`, `.zsh`, `.ps1`, `.bat`, `.cmd`)
- executable files and embedded archive payloads
- secret-like files (`.env`, private keys, credentials bundles)
- high-risk command snippets (for example pipe-to-shell payloads)
- prompt-injection override/exfiltration patterns
- phishing-style credential harvesting patterns
- obfuscated backdoor payload patterns (for example base64 decode-and-exec)
- markdown links that escape the skill root, point to remote markdown, or target script files

Use `skills audit` to manually validate a candidate skill directory (or an installed skill by name) before sharing it or trusting its source.

Skill manifests (`SKILL.toml`) support `prompts` and `[[tools]]`; both are injected into the agent system prompt at runtime, so the model can follow skill instructions without manually reading skill files.

### `migrate`

- `topclaw migrate openclaw [--source <path>] [--dry-run]`

### `config`

- `topclaw config schema`

`config schema` prints a JSON Schema (draft 2020-12) for the full `config.toml` contract to stdout.

### `completions`

- `topclaw completions bash`
- `topclaw completions fish`
- `topclaw completions zsh`
- `topclaw completions powershell`
- `topclaw completions elvish`

`completions` is stdout-only by design so scripts can be sourced directly without log/warning contamination.

### `hardware`

- `topclaw hardware discover`
- `topclaw hardware introspect <path>`
- `topclaw hardware info [--chip <chip_name>]`

### `peripheral`

- `topclaw peripheral list`
- `topclaw peripheral add <board> <path>`
- `topclaw peripheral flash [--port <serial_port>]`
- `topclaw peripheral setup-uno-q [--host <ip_or_host>]`
- `topclaw peripheral flash-nucleo`

## Validation Tip

To verify docs against your current binary quickly:

```bash
topclaw --help
topclaw <command> --help
```
