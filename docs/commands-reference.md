# TopClaw Commands Reference

This reference is derived from the current CLI surface (`topclaw --help`).

Last verified: **March 24, 2026**.

## Top-Level Commands

| Command | Purpose |
|---|---|
| `bootstrap` | Initialize workspace/config quickly or interactively |
| `agent` | Run interactive chat or single-message mode |
| `gateway` | Start webhook HTTP gateway |
| `daemon` | Start supervised runtime (gateway + channels + optional heartbeat/scheduler) |
| `service` | Manage user-level OS service lifecycle |
| `doctor` | Run diagnostics and freshness checks |
| `status` | Print current configuration and system summary |
| `update` | Check for or install the latest TopClaw release |
| `backup` | Create or restore a portable full-state backup bundle |
| `estop` | Engage/resume emergency stop levels and inspect estop state |
| `cron` | Manage scheduled tasks |
| `models` | Refresh provider model catalogs |
| `providers` | List provider IDs, aliases, and active provider |
| `channel` | Manage channels and channel health checks |
| `uninstall` | Remove TopClaw from this machine |
| `auth` | Manage provider authentication profiles |
| `security` | Manage security maintenance tasks |
| `memory` | Inspect and manage stored memory |
| `workspace` | Manage registered workspaces |
| `skills` | List/install/remove skills |
| `config` | Export machine-readable config schema |
| `completions` | Generate shell completion scripts to stdout |
| `hardware` | Discover, manage, and flash hardware boards |

Canonical command names:

- Use the command names above in docs, scripts, automation, and operator runbooks.
- Breaking release note: older CLI aliases such as `topclaw init`, `chat`, `run`, `info`, `check`, `channels`, and `skill` were removed.

## Most Common Commands

| If you want to... | Command |
|---|---|
| check whether TopClaw is ready | `topclaw status` |
| get the summary plus deeper diagnostics | `topclaw status --diagnose` |
| talk to TopClaw in this terminal | `topclaw agent` |
| test one prompt quickly | `topclaw agent -m "Hello, TopClaw!"` |
| check whether background channels are running | `topclaw service status` |
| start the background service manually | `topclaw service install`, `topclaw service start` |
| rerun onboarding | `topclaw bootstrap --interactive` |

If you only need the common day-1/day-2 commands, the table above is the fastest path. The rest of this page covers the full CLI surface.

## Command Groups

### `bootstrap`

- `topclaw bootstrap`
- `topclaw bootstrap --interactive`
- `topclaw bootstrap --channels-only`
- `topclaw bootstrap --force`
- `topclaw bootstrap --api-key <KEY> --provider <ID> --memory <sqlite|lucid|markdown|none>`
- `topclaw bootstrap --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none>`
- `topclaw bootstrap --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none> --force`
- `topclaw bootstrap --install-desktop-helpers`

`bootstrap` safety behavior:

- If `config.toml` already exists and you run `--interactive`, onboarding now offers two modes:
  - Full onboarding (overwrite `config.toml`)
  - Provider-only update (update provider/model/API key while preserving existing channels, tunnel, memory, hooks, and other settings)
- In non-interactive environments, existing `config.toml` causes a safe refusal unless `--force` is passed.
- Use `topclaw bootstrap --channels-only` when you only need to rotate channel tokens/allowlists.
- Use `topclaw bootstrap --install-desktop-helpers` to install missing Linux desktop automation helpers (xdotool, wmctrl, scrot, xdg-open) during headless/CI bootstrap. This is useful when the `desktop-computer-use` skill is needed but the system lacks the required X11 tools.

### `agent`

- `topclaw agent`
- `topclaw agent -m "Hello"`
- `topclaw agent --provider <ID> --model <MODEL> --temperature <0.0-2.0>`
- `topclaw agent --hardware <board:path>`

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

Use `service` for normal always-on channel operation. If onboarding already installed and started the service for you, begin with `topclaw service status` instead of reinstalling it.

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
curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/install-release.sh | bash
```

### `backup`

- `topclaw backup create <destination_dir>`
- `topclaw backup create <destination_dir> --include-logs`
- `topclaw backup inspect <source_dir>`
- `topclaw backup restore <source_dir>`
- `topclaw backup restore <source_dir> --force`

Notes:

- `backup create` exports the resolved TopClaw config root, including `config.toml`, auth state, secrets, memories, preferences, workspace data, and installed skills.
- `backup create` now records per-file checksums and writes a `RESTORE.md` guide into the bundle so moving it to another machine is less error-prone.
- `backup inspect` verifies the copied bundle before restore and prints the recorded file/byte totals.
- Runtime logs are excluded by default so the bundle stays smaller and more portable; add `--include-logs` if you want them.
- `backup restore` is designed for disaster recovery and machine migration. It restores into the current runtime config location and refreshes TopClaw's active-workspace marker.
- `backup restore` refuses to overwrite a non-empty target unless `--force` is passed.
- During `backup restore --force`, TopClaw moves the previous target config into a sibling rollback directory instead of deleting it first.
- If TopClaw is running as a background service, stop or restart the service around restore so the runtime picks up the recovered state cleanly.

### `cron`

- `topclaw cron list`
- `topclaw cron add <expr> [--tz <IANA_TZ>] <command>`
- `topclaw cron add-at <rfc3339_timestamp> <command>`
- `topclaw cron add-every <every_ms> <command>`
- `topclaw cron once <delay> <command>`
- `topclaw cron remove <id>`
- `topclaw cron update <id> [--expression <EXPR>] [--tz <TZ>] [--command <CMD>] [--name <NAME>]`
- `topclaw cron pause <id>`
- `topclaw cron resume <id>`

Notes:

- Mutating schedule/cron actions require `cron.enabled = true`.
- Shell command payloads for schedule creation (`create` / `add` / `once`) are validated by security command policy before job persistence.

### `models`

- `topclaw models refresh`
- `topclaw models refresh --provider <ID>`
- `topclaw models refresh --all`
- `topclaw models refresh --force`
- `topclaw models list [--provider <ID>]`
- `topclaw models set <MODEL>`
- `topclaw models status`

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
- `topclaw doctor desktop-helpers [--install]`
- `topclaw doctor models [--provider <ID>] [--use-cache]`
- `topclaw doctor traces [--limit <N>] [--event <TYPE>] [--contains <TEXT>]`
- `topclaw doctor traces --id <TRACE_ID>`

`topclaw doctor` now ends with concrete next-step commands when it detects actionable setup issues, such as missing provider configuration, missing auth, missing channels, or a missing workspace directory.

`topclaw doctor desktop-helpers` checks for missing Linux desktop automation helpers (xdotool, wmctrl, scrot, xdg-open) when `browser.backend` is set to `computer_use`. Pass `--install` to attempt automatic installation via the system package manager.

- When all helpers are present: prints `✅ All desktop helpers installed`.
- When helpers are missing: prints the list and suggests `topclaw doctor desktop-helpers --install`.
- When desktop automation is not configured: prints a hint to run `topclaw bootstrap --interactive` and select the `desktop-computer-use` skill.

Beginner guidance:

- prefer `topclaw status --diagnose` when you want the normal summary first
- use `topclaw doctor` when you want to jump straight into diagnostics

### `status`

- `topclaw status`
- `topclaw status --diagnose`

`topclaw status` prints the current config/runtime summary and readiness signals.

`topclaw status --diagnose` prints the same summary first, then the deeper `doctor` report and next-step commands.

When `browser.backend = "computer_use"` is configured, `topclaw status` includes a **Desktop** readiness line in the Readiness section:
- `✅ desktop helpers installed (xdotool, wmctrl, scrot, xdg-open)` — all helpers present
- `⚠️  missing: xdotool, wmctrl — run topclaw doctor desktop-helpers --install` — some helpers missing

Missing desktop helpers also cause the **Overall** readiness indicator to show `⚠️  action needed` instead of `✅ ready`, ensuring the status accurately reflects whether computer-use automation will work.

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

If you only need the most common channel/runtime checks, start with:

- `topclaw service status`
- `topclaw channel doctor`
- `topclaw channel start` only for deliberate foreground/manual troubleshooting

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

Normal runtime guidance:
- prefer `topclaw service ...` for always-on background channels
- use `topclaw channel start` when you explicitly want a manual foreground channel process

Channel runtime also watches `config.toml` and hot-applies updates to:
- `default_provider`
- `default_model`
- `default_temperature`
- `api_key` / `api_url` (for the default provider)
- `reliability.*` provider retry settings

`add/remove` currently route you back to managed setup/manual config paths (not full declarative mutators yet).

### `skills`

- `topclaw skills list`
- `topclaw skills vet <source_or_name> [--json] [--sandbox docker]`
- `topclaw skills audit <source_or_name>`
- `topclaw skills install <source>`
- `topclaw skills enable <name>`
- `topclaw skills disable <name>`
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
- Curated defaults stay transparent: reviewed TopClaw skill sources live under repository `/skills/`, and curated installs prefer a local TopClaw repo checkout when available.
- To pre-configure behavior, edit:
  - `aliases` (custom source shortcuts)
  - `trusted_domains`
  - `blocked_domains`

`skills list` now shows enabled and disabled skills separately. Disabling a skill moves it to `<workspace>/skills-disabled/` so it behaves like a plugin toggle rather than a delete; `skills enable` moves it back into `<workspace>/skills/`.

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

### `uninstall`

- `topclaw uninstall`
- `topclaw uninstall --purge`

`uninstall` removes background service artifacts and the installed TopClaw binary. Use `--purge` to also remove `~/.topclaw` config, logs, auth profiles, and workspace data.

### `auth`

- `topclaw auth login --provider <PROVIDER> [--profile <NAME>] [--device-code]`
- `topclaw auth paste-redirect --provider <PROVIDER> [--profile <NAME>] [--input <URL_OR_CODE>]`
- `topclaw auth paste-token --provider <PROVIDER> [--profile <NAME>] [--token <TOKEN>] [--auth-kind <KIND>]`
- `topclaw auth refresh --provider <PROVIDER> [--profile <NAME>]`
- `topclaw auth logout --provider <PROVIDER> [--profile <NAME>]`
- `topclaw auth use --provider <PROVIDER> --profile <NAME>`
- `topclaw auth list`
- `topclaw auth status`

Notes:

- `login` starts an OAuth flow for supported providers (for example `openai-codex`, `gemini`). Use `--device-code` for device-code flow.
- `paste-redirect` completes an OAuth flow by pasting the redirect URL or auth code.
- `paste-token` handles Anthropic subscription auth tokens.
- `refresh` refreshes an access token using a saved refresh token.
- `use` switches the active profile for a provider.
- `list` and `status` show saved profiles and active token expiry information.

### `security`

- `topclaw security update-guard-corpus [--source <SOURCE>] [--checksum <SHA256>]`

Notes:

- `update-guard-corpus` upserts semantic prompt-injection corpus records into the configured vector collection.
- `--source` accepts `builtin`, a filesystem path, or an HTTP(S) URL.
- `--checksum` provides SHA-256 hex verification for the source payload.

### `memory`

- `topclaw memory list [--category <CATEGORY>] [--session <ID>] [--limit <N>] [--offset <N>]`
- `topclaw memory get <KEY>`
- `topclaw memory stats`
- `topclaw memory clear [--key <KEY>] [--category <CATEGORY>] [--yes]`

Notes:

- `list` shows memory entries with optional category/session filtering and pagination.
- `get` retrieves a specific memory entry by key.
- `stats` shows memory backend statistics and health.
- `clear` removes memories by key, by category, or all entries. Use `--yes` to skip confirmation.

### `workspace`

- `topclaw workspace create [--name <NAME>]`
- `topclaw workspace list`
- `topclaw workspace disable <WORKSPACE_ID>`
- `topclaw workspace delete <WORKSPACE_ID> --confirm`
- `topclaw workspace token rotate <WORKSPACE_ID>`

Notes:

- The workspace registry is opt-in, controlled by `[workspaces].enabled = true` in config.
- `create` prints the workspace bearer token once at creation time.
- `token rotate` regenerates the bearer token for the specified workspace.
- `delete` requires `--confirm` to acknowledge destructive deletion.

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

- `topclaw hardware list`
- `topclaw hardware add <board> <path>`
- `topclaw hardware flash [--port <serial_port>]`
- `topclaw hardware setup-uno-q [--host <ip_or_host>]`
- `topclaw hardware flash-nucleo`

## Validation Tip

To verify docs against your current binary quickly:

```bash
topclaw --help
topclaw <command> --help
```
