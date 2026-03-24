# TopClaw Runtime Model

Use this page when you are unsure which runtime command you actually need after onboarding.

Last verified: **March 24, 2026**.

## The Short Version

| Goal | Command | Normal use |
|---|---|---|
| Talk to TopClaw directly in this terminal | `topclaw agent` | quick local chats and one-off prompts |
| Keep configured channels running in the background | `topclaw service install`, `topclaw service start`, `topclaw service status` | always-on Telegram, Discord, and similar setups |
| Run the full runtime in the foreground | `topclaw daemon` | debugging startup and watching live logs |
| Run only the HTTP/webhook gateway | `topclaw gateway` | webhook testing and gateway-only scenarios |
| Start channel listeners manually in one foreground process | `topclaw channel start` | advanced/manual troubleshooting |

## What Onboarding Does

After onboarding saves your config:

- provider auth should be ready, or TopClaw should tell you the exact next auth command
- your default model should already be selected
- chosen channels should already be written into `config.toml`
- when supported, TopClaw tries to install and start the background service automatically for channel-backed setups

That means most users should not need to guess between `topclaw daemon` and `topclaw service start`.

## Service Support By Environment

| Environment | Background service expectation |
|---|---|
| Linux with `systemd --user` | auto-managed when onboarding configures supported background channels |
| macOS with `launchd` | auto-managed when onboarding configures supported background channels |
| Windows service wrapper / scheduled runtime | auto-managed when onboarding configures supported background channels |
| OpenRC or other manual-only environments | manual setup may still be required |

## Which Command Should I Use?

### I just want to test the assistant right now

```bash
topclaw agent -m "Hello, TopClaw!"
```

### I configured Telegram, Discord, or another background channel

Start with:

```bash
topclaw service status
```

If it is not running yet:

```bash
topclaw service install
topclaw service start
```

If you are on an environment with manual-only service support, onboarding may stop after writing config and tell you to run the service commands yourself.

### I want to debug startup problems in the foreground

```bash
topclaw daemon
```

Use this when you want the runtime to stay attached to the terminal so you can see failures immediately.

### I only need the gateway HTTP surface

```bash
topclaw gateway
```

This is not the normal command for keeping chat channels alive.

### I am troubleshooting channels manually

```bash
topclaw channel doctor
topclaw channel start
```

`topclaw channel start` is mainly for debugging or manual foreground operation. For normal always-on use, prefer `topclaw service ...`.

## Recommended First Checks

After onboarding or after changing config:

```bash
topclaw status
topclaw status --diagnose
topclaw service status
```

## Common Confusion

### Do I need both `topclaw daemon` and `topclaw service install`?

No. Usually you need one runtime path:

- use `topclaw service ...` for background operation
- use `topclaw daemon` for foreground debugging

### Do I need `topclaw channel start` for normal channel use?

Usually no. Use it when you deliberately want a foreground/manual channel process.

### Why did `bootstrap.sh --prefer-prebuilt` not use my local code changes?

Because `--prefer-prebuilt` may install the latest released binary first. Use `./bootstrap.sh --force-source-build` when you need the checkout itself.
