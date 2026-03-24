# Channels Reference

This document is the canonical reference for channel configuration in TopClaw.

Last verified: **March 23, 2026**.

> **Implemented channels:** CLI, Telegram, Discord, Bridge, and Webhook.
> All other channel types (Slack, Mattermost, Matrix, etc.) were removed in v2026.3.22.

## Quick Paths

- Need a full config reference by channel: jump to [Per-Channel Config Examples](#4-per-channel-config-examples).
- Need a no-response diagnosis flow: jump to [Troubleshooting Checklist](#6-troubleshooting-checklist).
- Need deployment/network assumptions (polling vs webhook): use [Network Deployment](./network-deployment.md).

---

## 1. Configuration Namespace

All channel settings live under `channels_config` in `~/.topclaw/config.toml`.

```toml
[channels_config]
cli = true
```

Each channel is enabled by creating its sub-table (for example, `[channels_config.telegram]`).

One TopClaw runtime can serve multiple channels at once: if you configure several
channel sub-tables, `topclaw channel start` launches all of them in the same process.
Channel startup is best-effort: a single channel init failure is reported and skipped,
while remaining channels continue running.

## In-Chat Runtime Commands

When running `topclaw channel start` (or daemon mode), runtime commands include:

Telegram/Discord sender-scoped model routing:
- `/models` — show available providers and current selection
- `/models <provider>` — switch provider for the current sender session
- `/model` — show current model and cached model IDs (if available)
- `/model <model-id>` — switch model for the current sender session
- `/new` — clear conversation history and start a fresh session

Supervised tool approvals (all non-CLI channels):
- `/approve-request <tool-name>` — create a pending approval request
- `/approve-confirm <request-id>` — confirm pending request (same sender + same chat/channel only)
- `/approve-pending` — list pending requests for your current sender+chat/channel scope
- `/approve <tool-name>` — direct one-step approve + persist (`autonomy.auto_approve`, compatibility path)
- `/unapprove <tool-name>` — revoke and remove persisted approval
- `/approvals` — inspect runtime grants, persisted approval lists, and excluded tools

Notes:

- Switching provider or model clears only that sender's in-memory conversation history to avoid cross-model context contamination.
- `/new` clears the sender's conversation history without changing provider or model selection.
- Model cache previews come from `topclaw models refresh --provider <ID>`.
- These are runtime chat commands, not CLI subcommands.
- Natural-language approval intents are supported with strict parsing and policy control:
  - `direct` mode (default): `授权工具 shell` grants immediately.
  - `request_confirm` mode: `授权工具 shell` creates pending request, then confirm with request ID.
  - `disabled` mode: approval-management must use slash commands.
- You can override natural-language approval mode per channel via `[autonomy].non_cli_natural_language_approval_mode_by_channel`.
- Approval commands are intercepted before LLM execution, so the model cannot self-escalate permissions through tool calls.
- You can restrict who can use approval-management commands via `[autonomy].non_cli_approval_approvers`.
- Configure natural-language approval mode via `[autonomy].non_cli_natural_language_approval_mode`.
- `autonomy.non_cli_excluded_tools` is reloaded from `config.toml` at runtime; `/approvals` shows the currently effective list.
- Each incoming message injects a runtime tool-availability snapshot into the system prompt, derived from the same exclusion policy used by execution.

## Inbound Image Marker Protocol

TopClaw supports multimodal input through inline message markers:

- Syntax: ``[IMAGE:<source>]``
- `<source>` can be:
  - Local file path
  - Data URI (`data:image/...;base64,...`)
  - Remote URL only when `[multimodal].allow_remote_fetch = true`

Operational notes:

- Marker parsing applies to user-role messages before provider calls.
- Provider capability is enforced at runtime: if the selected provider does not support vision, the request fails with a structured capability error (`capability=vision`).

## Channel Matrix

### Build Feature Toggles (`channel-telegram`, `channel-discord`)

Channel support is controlled at compile time.

- Default builds include Telegram (`default = ["channel-telegram"]`), while Discord is opt-in.
- For a lean local build without any channel features:

```bash
cargo check --no-default-features --features hardware
```

- Enable Discord explicitly in a custom feature set:

```bash
cargo check --no-default-features --features hardware,channel-discord
```

If a channel sub-table is present in config but the corresponding feature is not compiled in, `topclaw channel list`, `topclaw channel doctor`, and `topclaw channel start` will report that the channel is intentionally skipped for this build.

---

## 2. Delivery Modes at a Glance

| Channel | Receive mode | Public inbound port required? |
|---|---|---|
| CLI | local stdin/stdout | No |
| Telegram | polling | No |
| Discord | gateway/websocket | No |
| Bridge | external bridge process | No |
| Webhook | gateway endpoint (`/webhook`) | Usually yes |

---

## 3. Allowlist Semantics

For channels with inbound sender allowlists:

- Empty allowlist: deny all inbound messages.
- `"*"`: allow all inbound senders (use for temporary verification only).
- Explicit list: allow only listed senders.

Field name: `allowed_users` (Telegram, Discord).

### Group-Chat Trigger Policy (Telegram/Discord)

These channels support an explicit `group_reply` policy:

- `mode = "all_messages"`: reply to all group messages (subject to channel allowlist checks).
- `mode = "mention_only"`: in groups, require explicit bot mention.
- `allowed_sender_ids`: sender IDs that bypass mention gating in groups.

Important behavior:

- `allowed_sender_ids` only bypasses mention gating.
- Sender allowlists (`allowed_users`) are still enforced first.

Example shape:

```toml
[channels_config.telegram.group_reply]
mode = "mention_only"                      # all_messages | mention_only
allowed_sender_ids = ["123456789", "987"] # optional; "*" allowed
```

---

## 4. Per-Channel Config Examples

### 4.1 Telegram

```toml
[channels_config.telegram]
bot_token = "123456:telegram-token"
allowed_users = ["*"]
stream_mode = "off"               # optional: off | partial
draft_update_interval_ms = 500    # optional: edit throttle for partial streaming
interrupt_on_new_message = false  # optional: cancel in-flight same-sender same-chat request

[channels_config.telegram.group_reply]
mode = "all_messages"             # optional: all_messages | mention_only
allowed_sender_ids = []           # optional: sender IDs that bypass mention gate
```

Telegram notes:

- `interrupt_on_new_message = true` preserves interrupted user turns in conversation history, then restarts generation on the newest message.
- Interruption scope is strict: same sender in the same chat. Messages from different chats are processed independently.

### 4.2 Discord

```toml
[channels_config.discord]
bot_token = "discord-bot-token"
guild_id = "123456789012345678"   # optional
allowed_users = ["*"]
listen_to_bots = false

[channels_config.discord.group_reply]
mode = "all_messages"             # optional: all_messages | mention_only
allowed_sender_ids = []           # optional: sender IDs that bypass mention gate
```

### 4.3 Webhook (Gateway)

`channels_config.webhook` enables webhook-specific gateway behavior.

```toml
[channels_config.webhook]
port = 8080
secret = "optional-shared-secret"
```

Run with gateway/daemon and verify `/health`.

---

## 5. Validation Workflow

1. Configure one channel with permissive allowlist (`"*"`) for initial verification.
2. Run:

```bash
topclaw bootstrap --channels-only
topclaw daemon
```

1. Send a message from an expected sender.
2. Confirm a reply arrives.
3. Tighten allowlist from `"*"` to explicit IDs.

---

## 6. Troubleshooting Checklist

If a channel appears connected but does not respond:

1. Confirm the sender identity is allowed by the correct allowlist field.
2. Confirm bot account membership/permissions in target room/channel.
3. Confirm tokens/secrets are valid (and not expired/revoked).
4. Confirm transport mode assumptions:
   - polling/websocket channels do not need public inbound HTTP
   - webhook channels do need reachable HTTPS callback
5. Restart `topclaw daemon` after config changes.


---

## 7. Operations Appendix: Log Keywords Matrix

Use this appendix for fast triage. Match log keywords first, then follow the troubleshooting steps above.

### 7.1 Recommended capture command

```bash
RUST_LOG=info topclaw daemon 2>&1 | tee /tmp/topclaw.log
```

Then filter channel/gateway events:

```bash
rg -n "Telegram|Discord|Webhook|Channel" /tmp/topclaw.log
```

### 7.2 Keyword table

| Component | Startup / healthy signal | Authorization / policy signal | Transport / failure signal |
|---|---|---|---|
| Telegram | `Telegram channel listening for messages...` | `Telegram: ignoring message from unauthorized user:` | `Telegram poll error:` / `Telegram parse error:` / `Telegram polling conflict (409):` |
| Discord | `Discord: connected and identified` | `Discord: ignoring message from unauthorized user:` | `Discord: received Reconnect (op 7)` / `Discord: received Invalid Session (op 9)` |
| Webhook (gateway) | gateway startup log | `Webhook: rejected — not paired / invalid bearer token` / `Webhook: rejected request — invalid or missing X-Webhook-Secret` | `Webhook JSON parse error:` |

### 7.3 Runtime supervisor keywords

If a specific channel task crashes or exits, the channel supervisor in `channels/mod.rs` emits:

- `Channel <name> exited unexpectedly; restarting`
- `Channel <name> error: ...; restarting`
- `Channel message worker crashed:`

These messages indicate automatic restart behavior is active, and you should inspect preceding logs for root cause.
