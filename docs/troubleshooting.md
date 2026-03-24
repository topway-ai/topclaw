# TopClaw Troubleshooting

This guide focuses on common setup/runtime failures and fast resolution paths.

Last verified: **March 24, 2026**.

## Quick Triage

| If this happened | Start here |
|---|---|
| `topclaw` command not found after install | [`topclaw` command not found after install](#topclaw-command-not-found-after-install) |
| Onboarding finished but replies do not work | [Onboarding finished but TopClaw still does not reply](#onboarding-finished-but-topclaw-still-does-not-reply) |
| Channels are configured but background runtime is not running | [Service installed but not running](#service-installed-but-not-running) |
| Provider auth is still missing or expired | [Provider auth missing or expired](#provider-auth-missing-or-expired) |

## Installation / Bootstrap

### `cargo` not found

Symptom:

- bootstrap exits with `cargo is not installed`

Fix:

```bash
./bootstrap.sh --install-rust
```

Or install from <https://rustup.rs/>.

### Missing system build dependencies

Symptom:

- build fails due to compiler or `pkg-config` issues

Fix:

```bash
./bootstrap.sh --install-system-deps
```

### Build fails on low-RAM / low-disk hosts

Symptoms:

- `cargo build --release` is killed (`signal: 9`, OOM killer, or `cannot allocate memory`)
- Build crashes after adding swap because disk space runs out

Why this happens:

- Runtime memory (<5MB for common operations) is not the same as compile-time memory.
- Full source build can require **2 GB RAM + swap** and **6+ GB free disk**.
- Enabling swap on a tiny disk can avoid RAM OOM but still fail due to disk exhaustion.

Preferred path for constrained machines:

```bash
./bootstrap.sh --prefer-prebuilt
```

Binary-only mode (no source fallback):

```bash
./bootstrap.sh --prebuilt-only
```

If you must compile from source on constrained hosts:

1. Add swap only if you also have enough free disk for both swap + build output.
1. Limit cargo parallelism:

```bash
CARGO_BUILD_JOBS=1 cargo build --release --locked
```

1. Reduce features for a leaner build:

```bash
cargo build --release --locked --no-default-features --features hardware
```

1. Cross-compile on a stronger machine and copy the binary to the target host.

### Build is very slow or appears stuck

Symptoms:

- `cargo check` / `cargo build` appears stuck at `Checking topclaw` for a long time
- repeated `Blocking waiting for file lock on package cache` or `build directory`

Why this happens in TopClaw:

- TLS + crypto native build scripts (`aws-lc-sys`, `ring`) add noticeable compile time.
- `rusqlite` with bundled SQLite compiles C code locally.
- Running multiple cargo jobs/worktrees in parallel causes lock contention.

Fast checks:

```bash
cargo check --timings
cargo tree -d
```

The timing report is written to `target/cargo-timings/cargo-timing.html`.

Faster local iteration (lean default feature set):

```bash
cargo check
```

This uses the lean default feature set (`channel-telegram` only) and can significantly reduce compile time.

To build with Discord + hardware support:

```bash
cargo check --features channel-discord,hardware
```

Lock-contention mitigation:

```bash
pgrep -af "cargo (check|build|test)|cargo check|cargo build|cargo test"
```

Stop unrelated cargo jobs before running your own build.

### `topclaw` command not found after install

Symptom:

- install succeeds but shell cannot find `topclaw`

Fix:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
which topclaw
```

Persist in your shell profile if needed.

### Onboarding finished but TopClaw still does not reply

Check in this order:

```bash
topclaw status
topclaw status --diagnose
topclaw service status
topclaw channel doctor
```

What to look for:

- provider auth still missing
- service not installed or not running
- channel credentials or allowlists still incomplete
- onboarding completed on a platform where service setup is manual

If `topclaw service status` reports that nothing is running yet:

```bash
topclaw service install
topclaw service start
```

If you want to debug startup in the foreground instead:

```bash
topclaw daemon
```

## Runtime / Gateway

### Gateway unreachable

Checks:

```bash
topclaw status
topclaw doctor
```

Verify `~/.topclaw/config.toml`:

- `[gateway].host` (default `127.0.0.1`)
- `[gateway].port` (default `42617`)
- `allow_public_bind` only when intentionally exposing LAN/public interfaces

### Provider auth missing or expired

Checks:

```bash
topclaw status
topclaw status --diagnose
```

What this usually means:

- your provider still needs OAuth/subscription login
- your API key was never set
- your saved auth expired and needs to be refreshed

Fix:

1. Read the next-step auth command shown by `topclaw status --diagnose`.
2. Complete the provider login flow or set the correct API key.
3. Run `topclaw status` again to confirm the provider is ready.

### Pairing / auth failures on webhook

Checks:

1. Ensure pairing completed (`/pair` flow)
2. Ensure bearer token is current
3. Re-run diagnostics:

```bash
topclaw status --diagnose
```

## Channel Issues

### Telegram conflict: `terminated by other getUpdates request`

Cause:

- multiple pollers using same bot token

Fix:

- keep only one active runtime for that token
- stop extra `topclaw daemon` / `topclaw channel start` processes

### Channel unhealthy in `channel doctor`

Checks:

```bash
topclaw channel doctor
```

Then verify channel-specific credentials + allowlist fields in config.

If the channel is configured correctly but still does not reply, confirm that a runtime is actually active. For normal use, prefer `topclaw service status` over `topclaw channel start`.

## Service Mode

To keep TopClaw running in the background all the time:

```bash
topclaw service install
topclaw service start
topclaw service status
```

To restart it cleanly:

```bash
topclaw service stop
topclaw service start
```

### Service installed but not running

Checks:

```bash
topclaw service status
```

Recovery:

```bash
topclaw service stop
topclaw service start
```

Linux logs:

```bash
journalctl --user -u topclaw.service -f
```

If onboarding just finished and you are unsure which command to use next, read [runtime-model.md](runtime-model.md) before starting `daemon` or `channel start` manually.

## Installer Compatibility

Use the canonical hosted installer:

```bash
curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/bootstrap.sh | bash
```

## Still Stuck?

Collect and include these outputs when filing an issue:

```bash
topclaw --version
topclaw status
topclaw doctor
topclaw channel doctor
```

Also include OS, install method, and sanitized config snippets (no secrets).

## Related Docs

- [operations-runbook.md](operations-runbook.md)
- [one-click-bootstrap.md](one-click-bootstrap.md)
- [channels-reference.md](channels-reference.md)
- [network-deployment.md](network-deployment.md)
