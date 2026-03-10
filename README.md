# TopClaw

TopClaw is a Rust-based AI agent runtime for local and remote AI workflows.

## Quick Start

If you are on macOS, install Apple developer tools first:

```bash
xcode-select --install
```

Then use the same quick-start path on Linux or macOS:

```bash
git clone https://github.com/topway-ai/TopClaw.git
cd TopClaw
./bootstrap.sh --install-system-deps --install-rust --prefer-prebuilt
topclaw status
topclaw status --diagnose
topclaw agent -m "Hello, TopClaw!"
```

This path installs standard prerequisites, installs Rust when missing, prefers a prebuilt binary first, and starts onboarding automatically.

If you need to test the local checkout instead of the latest release asset, use `./bootstrap.sh --force-source-build`.

## What Bootstrap Does

Recommended first-run command:

```bash
./bootstrap.sh --install-system-deps --install-rust --prefer-prebuilt
```

What those flags do:

1. install missing system dependencies when possible
2. install Rust if it is not already present
3. try a prebuilt `topclaw` binary first, then fall back to source build if needed
4. start the onboarding wizard

Important:

- `--prefer-prebuilt` may install the latest released TopClaw binary, not the exact code in your checkout
- use `--force-source-build` when you need to validate local source changes

During onboarding, the default path is now:

- choose your AI provider
- authenticate or enter the provider API key if needed
- choose a channel such as Telegram or Discord
- enter the channel token and allowed user info
- let onboarding try to start the background service automatically when your selected channels need one

Everything else can be changed later in `config.toml`.

After onboarding, `topclaw status` should show whether the provider is ready, whether channels are configured, and whether any manual action is still required.

## Fast Path

If you already know which auth path you need:

API key providers:

```bash
topclaw onboard --api-key "sk-..." --provider openrouter
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

Use `topclaw gateway` only when you are intentionally testing the HTTP/webhook surface.

## Runtime Modes

TopClaw has a few different runtime commands:

- `topclaw agent`: talk to TopClaw directly in this terminal
- `topclaw service ...`: keep configured channels running in the background
- `topclaw daemon`: run the full runtime in the foreground for debugging
- `topclaw gateway`: run only the HTTP/webhook gateway

For the full explanation, see [`docs/runtime-model.md`](docs/runtime-model.md).

## Run In Background

If onboarding configured background channels on a supported platform, it now tries to install and start the service automatically.

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

- Getting started: [`docs/getting-started/README.md`](docs/getting-started/README.md)
- Commands and config: [`docs/reference/README.md`](docs/reference/README.md)
- Operations and troubleshooting: [`docs/operations/README.md`](docs/operations/README.md)
- Full docs hub: [`docs/README.md`](docs/README.md)

### Other Platforms

- Windows: run `.\bootstrap.ps1`
- Lower-resource machines: `./bootstrap.sh --prefer-prebuilt`
