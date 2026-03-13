# Getting Started

Use this section for first install, onboarding, and first-run validation.

## Recommended Path

1. Run the main quick start: [../../README.md](../../README.md)
2. Run the hosted installer for your shell:
   `curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/bootstrap.sh | bash`
   or on Windows PowerShell:
   `iwr -useb https://raw.githubusercontent.com/topway-ai/topclaw/main/bootstrap.ps1 | iex`
3. Let the installer try the latest compatible release asset first, fall back to a source build only if needed, and launch onboarding
4. Validate readiness with `topclaw status`
5. Run deeper checks with `topclaw status --diagnose`
6. Send a test prompt with `topclaw agent -m "Hello, TopClaw!"`

If you need to review the installer first or test the local checkout instead of the latest release asset, clone the repo and use `./bootstrap.sh --force-source-build`.

## Setup Paths

| Situation | Recommended path |
|---|---|
| New user on Linux or macOS | [../../README.md](../../README.md) |
| New user on Windows | `iwr -useb https://raw.githubusercontent.com/topway-ai/topclaw/main/bootstrap.ps1 | iex` |
| Want bootstrap details and flags | [../one-click-bootstrap.md](../one-click-bootstrap.md) |
| Unsure whether to use `agent`, `service`, or `daemon` | [../runtime-model.md](../runtime-model.md) |
| Already have an API key | `topclaw bootstrap --api-key "sk-..." --provider openrouter` |
| Need OAuth or subscription login instead of an API key | Use `topclaw bootstrap --interactive` and follow the provider login prompt |
| Want interactive provider and channel setup | `topclaw bootstrap --interactive` |
| Need Android/Termux instructions | [../android-setup.md](../android-setup.md) |
| Need macOS update or uninstall steps | [macos-update-uninstall.md](macos-update-uninstall.md) |

## Validation

After setup, the fastest checks are:

```bash
topclaw status
topclaw status --diagnose
topclaw service status
topclaw agent -m "Hello, TopClaw!"
```

`topclaw status --diagnose` is the recommended beginner check because it shows the normal status summary first and then the deeper diagnostics. `topclaw doctor` remains the direct diagnostics command underneath.

## What Success Looks Like

After onboarding, you should have:

- a ready provider or a single explicit auth command to run next
- a selected default model
- configured channels saved into `config.toml`
- a background service already running when your chosen channels require one and your platform supports automatic setup

## Next

- Commands: [../commands-reference.md](../commands-reference.md)
- Config: [../config-reference.md](../config-reference.md)
- Providers and channels: [../reference/README.md](../reference/README.md)
- Operations and troubleshooting: [../operations/README.md](../operations/README.md)
