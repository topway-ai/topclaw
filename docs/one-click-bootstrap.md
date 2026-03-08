# One-Click Bootstrap

This page defines the fastest supported path to install and initialize TopClaw.

Last verified: **March 7, 2026**.

## Safe update

For existing installs, the simplest supported update path is:

```bash
topclaw update
topclaw --version
```

To check first without installing:

```bash
topclaw update --check
```

If TopClaw runs as a background service, restart it after the update:

```bash
topclaw service restart
```

If `topclaw update` reports that the current binary location is not writable, fall back to the original install method:

- repo checkout installs: `./bootstrap.sh --prefer-prebuilt`
- source installs: `cargo install --path . --force --locked`
- package-manager installs: update through that package manager

## Option 0: Homebrew (macOS/Linuxbrew)

```bash
brew install topclaw
```

## Option A (Recommended): Clone + local script

```bash
git clone https://github.com/jackfly8/TopClaw.git
cd TopClaw
./bootstrap.sh --install-system-deps --install-rust --prefer-prebuilt
```

Windows PowerShell equivalent:

```powershell
git clone https://github.com/jackfly8/TopClaw.git
cd TopClaw
.\bootstrap.ps1 -InstallRust -PreferPrebuilt
```

What this recommended path does:

1. installs standard prerequisites when supported
2. installs Rust when missing
3. tries a prebuilt binary first
4. falls back to source build only if no compatible release asset exists

### Resource preflight and pre-built flow

Source builds typically require at least:

- **2 GB RAM + swap**
- **6 GB free disk**

When resources are constrained, bootstrap now attempts a pre-built binary first.

```bash
./bootstrap.sh --prefer-prebuilt
```

To require binary-only installation and fail if no compatible release asset exists:

```bash
./bootstrap.sh --prebuilt-only
```

To bypass pre-built flow and force source compilation:

```bash
./bootstrap.sh --force-source-build
```

## Dual-mode bootstrap

Default behavior is **app-only** (build/install TopClaw) and expects existing Rust toolchain.

For fresh machines, enable environment bootstrap explicitly:

```bash
./bootstrap.sh --install-system-deps --install-rust
```

Notes:

- `--install-system-deps` installs compiler/build prerequisites (may require `sudo`).
- `--install-rust` installs Rust via `rustup` when missing.
- `--prefer-prebuilt` tries release binary download first, then falls back to source build.
- `--prebuilt-only` disables source fallback.
- `--force-source-build` disables pre-built flow entirely.
- On Windows, use `bootstrap.ps1` (`-InstallRust`, `-PreferPrebuilt`, `-PrebuiltOnly`, `-ForceSourceBuild`).

## Option B: Remote one-liner

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/bootstrap.sh | bash
```

For high-security environments, prefer Option A so you can review the script before execution.

Legacy compatibility:

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/install.sh | bash
```

This legacy endpoint prefers forwarding to `scripts/bootstrap.sh` and falls back to legacy source install if unavailable in that revision.

If you run Option B outside a repository checkout, the bootstrap script automatically clones a temporary workspace, builds, installs, and then cleans it up.

## Optional onboarding modes

### Containerized onboarding (Docker)

```bash
./bootstrap.sh --docker
```

This builds a local TopClaw image and launches onboarding inside a container while
persisting config/workspace to `./.topclaw-docker`.

Container CLI defaults to `docker`. If Docker CLI is unavailable and `podman` exists,
bootstrap auto-falls back to `podman`. You can also set `TOPCLAW_CONTAINER_CLI`
explicitly (for example: `TOPCLAW_CONTAINER_CLI=podman ./bootstrap.sh --docker`).

For Podman, bootstrap runs with `--userns keep-id` and `:Z` volume labels so
workspace/config mounts remain writable inside the container.

If you add `--skip-build`, bootstrap skips local image build. It first tries the local
Docker tag (`TOPCLAW_DOCKER_IMAGE`, default: `topclaw-bootstrap:local`); if missing,
it pulls `ghcr.io/jackfly8/TopClaw:latest` and tags it locally before running.

### Quick onboarding (non-interactive)

```bash
./bootstrap.sh --onboard --api-key "sk-..." --provider openrouter
```

Or with environment variables:

```bash
TOPCLAW_API_KEY="sk-..." TOPCLAW_PROVIDER="openrouter" ./bootstrap.sh --onboard
```

### Interactive onboarding

```bash
./bootstrap.sh --interactive-onboard
```

## Useful flags

- `--install-system-deps`
- `--install-rust`
- `--skip-build` (in `--docker` mode: use local image if present, otherwise pull `ghcr.io/jackfly8/TopClaw:latest`)
- `--skip-install`
- `--provider <id>`

See all options:

```bash
./bootstrap.sh --help
```

## Related docs

- [README.md](../README.md)
- [commands-reference.md](commands-reference.md)
- [providers-reference.md](providers-reference.md)
- [channels-reference.md](channels-reference.md)
