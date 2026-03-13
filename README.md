# TopClaw

TopClaw is a Rust-based AI agent runtime for local and remote AI workflows.

## What TopClaw Is

TopClaw combines several runtime surfaces in one codebase:

- a CLI for setup, diagnostics, and direct chat
- an agent loop that can call tools and persist memory
- provider adapters for multiple model APIs
- channel adapters for Telegram, Discord, Slack, and others
- an HTTP, WebSocket, and OpenAI-compatible gateway
- optional hardware and peripheral integrations

The implementation is trait-driven. Most extensions are added by implementing an existing trait and registering it in the matching factory.

## Quick Start

Use the supported one-line installer for your shell.

Linux / macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/bootstrap.sh | bash
```

Windows PowerShell:

```powershell
iwr -useb https://raw.githubusercontent.com/topway-ai/topclaw/main/bootstrap.ps1 | iex
```

The hosted installers prefer the latest compatible release asset first, fall back to a source build only if needed, and then start onboarding automatically.

If you need to review the installer first or validate local source changes, use a repository checkout instead:

```bash
git clone https://github.com/topway-ai/topclaw.git
cd topclaw
./bootstrap.sh --install-system-deps --install-rust --prefer-prebuilt
```

Use `./bootstrap.sh --force-source-build` when you need to validate the local checkout instead of the latest release asset.

## What Bootstrap Does

The default onboarding path is:

1. choose your AI provider
2. authenticate or enter the provider API key if needed
3. choose a channel such as Telegram or Discord
4. enter the channel token and allowed user info
5. let onboarding try to install and start the background service when your selected channels need one

After onboarding, `topclaw status` shows whether the provider is ready, whether channels are configured, and whether any manual action is still required.

Important:

- the hosted installers prefer the latest released TopClaw binary, not the exact code in a local checkout
- use a local checkout plus `./bootstrap.sh --force-source-build` when you need to validate local source changes

## Fast Path

If you already know which auth path you need:

API key providers:

```bash
topclaw bootstrap --api-key "sk-..." --provider openrouter
```

OAuth or subscription providers:

- choose the provider during interactive onboarding and follow the login prompt
- if needed later, run the provider-specific auth command that `topclaw status` or onboarding shows you next

## First Commands

After onboarding, these are the most useful first commands:

```bash
topclaw status
topclaw status --diagnose
topclaw agent -m "Hello, TopClaw!"
```

If you configured a channel and the service is running, the first real end-to-end check is usually to send TopClaw a message in that channel.

Use `topclaw gateway` only when you are intentionally testing the HTTP or webhook surface.

## Runtime Modes

TopClaw exposes a few main runtime commands:

- `topclaw agent`: talk to TopClaw directly in this terminal
- `topclaw service ...`: keep configured channels running in the background
- `topclaw daemon`: run the full runtime in the foreground for debugging
- `topclaw gateway`: run only the HTTP, WebSocket, and webhook gateway

For the full explanation, see [`docs/runtime-model.md`](docs/runtime-model.md).

## Run In Background

If onboarding configured background channels on a supported platform, it tries to install and start the service automatically.

To confirm that background runtime is healthy:

```bash
topclaw service status
```

If the service still needs manual setup:

```bash
topclaw service install
topclaw service start
```

To stop the background service completely:

```bash
topclaw service stop
```

## Architecture At A Glance

TopClaw's main execution flow is:

1. load and normalize `config.toml`
2. build the runtime adapter, security policy, memory backend, observer, provider, and tool registry
3. accept input from the CLI, a channel listener, or the gateway
4. construct the system prompt and conversation history
5. ask the configured provider for a response
6. if the response contains tool calls, validate and execute them, then continue the loop
7. persist memory, emit observability events, and deliver the final response back to the caller

Important code-level constraints:

- security is policy-driven and deny-first for tool execution, shell access, and network exposure
- config and CLI behavior are public contracts, not internal details
- most extension points are narrow traits: `Provider`, `Channel`, `Tool`, `Memory`, `Observer`, `RuntimeAdapter`, and `Peripheral`
- several capabilities are feature-gated at compile time, including Matrix, WhatsApp Web, OpenTelemetry, hardware discovery, and the WASM runtime

## Repository Structure

High-signal paths:

- `src/main.rs`: CLI entrypoint and command routing
- `src/lib.rs`: crate exports and public command enums
- `src/agent/`: orchestration loop, prompt construction, dispatch, and research
- `src/config/`: config schema, defaults, loading, and validation
- `src/providers/`: model provider implementations and factories
- `src/channels/`: chat platform integrations
- `src/tools/`: agent-callable tools
- `src/memory/`: memory traits and backends
- `src/security/`: policy, pairing, and secrets
- `src/gateway/`: HTTP, SSE, WebSocket, and OpenAI-compatible endpoints
- `src/runtime/`: runtime adapters
- `src/peripherals/` and `src/hardware/`: hardware-facing integrations
- `examples/`: custom provider, tool, channel, and memory examples
- `tests/`: integration and regression coverage
- `docs/`: user-facing reference, operations, security, and architecture docs

## Extension Surfaces

If you are extending TopClaw, start with these contracts:

- providers: [`src/providers/traits.rs`](src/providers/traits.rs)
- channels: [`src/channels/traits.rs`](src/channels/traits.rs)
- tools: [`src/tools/traits.rs`](src/tools/traits.rs)
- memory backends: [`src/memory/traits.rs`](src/memory/traits.rs)
- observability backends: [`src/observability/traits.rs`](src/observability/traits.rs)
- runtimes: [`src/runtime/traits.rs`](src/runtime/traits.rs)
- peripherals: [`src/peripherals/traits.rs`](src/peripherals/traits.rs)

## Uninstall

To remove the TopClaw binary but keep your `~/.topclaw` data:

```bash
topclaw uninstall
```

To remove TopClaw completely, including `~/.topclaw` config, logs, auth profiles, and workspace data:

```bash
topclaw uninstall --purge
```

## Documentation Map

- getting started: [`docs/getting-started/README.md`](docs/getting-started/README.md)
- commands and config: [`docs/reference/README.md`](docs/reference/README.md)
- operations and troubleshooting: [`docs/operations/README.md`](docs/operations/README.md)
- full docs hub: [`docs/README.md`](docs/README.md)
