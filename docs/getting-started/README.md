# Getting Started

Use this section for first install, onboarding, and first-run validation.

## Recommended Path

1. Run the main quick start: [../../README.md](../../README.md)
2. Let `./bootstrap.sh` install prerequisites and launch onboarding
3. Validate with `topclaw status`
4. Send a test prompt with `topclaw agent -m "Hello!"`

## Setup Paths

| Situation | Recommended path |
|---|---|
| New user on Linux or macOS | [../../README.md](../../README.md) |
| Want bootstrap details and flags | [../one-click-bootstrap.md](../one-click-bootstrap.md) |
| Already have an API key | `topclaw onboard --api-key "sk-..." --provider openrouter` |
| Want interactive provider and channel setup | `topclaw onboard --interactive` |
| Need Android/Termux instructions | [../android-setup.md](../android-setup.md) |
| Need macOS update or uninstall steps | [macos-update-uninstall.md](macos-update-uninstall.md) |

## Validation

After setup, the fastest checks are:

```bash
topclaw status
topclaw doctor
topclaw agent -m "Hello!"
```

## Next

- Commands: [../commands-reference.md](../commands-reference.md)
- Config: [../config-reference.md](../config-reference.md)
- Providers and channels: [../reference/README.md](../reference/README.md)
- Operations and troubleshooting: [../operations/README.md](../operations/README.md)
