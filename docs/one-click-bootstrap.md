# One-Click Bootstrap

This page defines the fastest supported path to install and initialize TopClaw.

Last verified: **March 12, 2026**.

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

- hosted installer installs: `curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/bootstrap.sh | bash`
- source installs: `cargo install --path . --force --locked`
- package-manager installs: update through that package manager

## Option 0: Homebrew (macOS/Linuxbrew)

```bash
brew install topclaw
```

## Option A (Recommended): Hosted one-liner

```bash
curl -fsSL https://raw.githubusercontent.com/topway-ai/topclaw/main/scripts/bootstrap.sh | bash
```

Windows PowerShell equivalent:

```powershell
iwr -useb https://raw.githubusercontent.com/topway-ai/topclaw/main/bootstrap.ps1 | iex
```

What this recommended path does:

1. installs standard prerequisites when supported
2. installs Rust when missing
3. tries a prebuilt binary first
4. clones and falls back to source build only if no compatible release asset exists

Important:

- `--prefer-prebuilt` may install the latest released TopClaw binary instead of building the exact checkout in your current repository
- use `--force-source-build` when you need to validate local code changes

For high-security environments, prefer the repo checkout path below so you can review the script before execution.

## Option B: Clone + local script

```bash
git clone https://github.com/topway-ai/topclaw.git
cd topclaw
./bootstrap.sh --install-system-deps --install-rust --prefer-prebuilt
```

Windows PowerShell equivalent from a checkout:

```powershell
.\bootstrap.ps1 -InstallRust -PreferPrebuilt
```

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

If you run Option A outside a repository checkout, the bootstrap script now tries the latest compatible release asset without cloning first. It only clones a temporary workspace if it needs a source-build fallback, then cleans that workspace up afterward.

## Optional onboarding modes

### Containerized onboarding (Docker)

```bash
./bootstrap.sh --docker
```

This builds a local TopClaw image and launches onboarding inside a container while
persisting config/workspace to `./.topclaw-docker`.

Inside the container, TopClaw now runs `topclaw bootstrap` as the canonical setup
command. For the current release, use `bootstrap` only.

Container CLI defaults to `docker`. If Docker CLI is unavailable and `podman` exists,
bootstrap auto-falls back to `podman`. You can also set `TOPCLAW_CONTAINER_CLI`
explicitly (for example: `TOPCLAW_CONTAINER_CLI=podman ./bootstrap.sh --docker`).

For Podman, bootstrap runs with `--userns keep-id` and `:Z` volume labels so
workspace/config mounts remain writable inside the container.

If you add `--skip-build`, bootstrap skips local image build. It first tries the local
Docker tag (`TOPCLAW_DOCKER_IMAGE`, default: `topclaw-bootstrap:local`); if missing,
it pulls `ghcr.io/topway-ai/topclaw:latest` and tags it locally before running.

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
- `--skip-build` (in `--docker` mode: use local image if present, otherwise pull `ghcr.io/topway-ai/topclaw:latest`)
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
