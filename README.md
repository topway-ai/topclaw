# TopClaw

TopClaw is a Rust-first agent runtime for local and remote AI workflows.

## Start Here

If you want the fastest path:

1. Install Rust.
2. Clone or open the TopClaw repository.
3. Run:

```bash
./bootstrap.sh
```

`./bootstrap.sh` installs missing prerequisites when possible, builds `topclaw`, and starts onboarding.

## Setup Paths

### Linux

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev ca-certificates curl git

curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"

./bootstrap.sh
topclaw status
topclaw agent -m "Hello!"
```

### macOS

```bash
xcode-select --install
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"

./bootstrap.sh
topclaw status
topclaw agent -m "Hello!"
```

### Fast Non-Interactive Setup

If you already know your provider and API key:

```bash
topclaw onboard --api-key "sk-..." --provider openrouter
```

## What Onboarding Covers

The default onboarding flow is intentionally short:

- choose a provider
- enter an API key if needed
- choose a channel
- enter the channel token and allowed user info

Everything else can be changed later in `config.toml`.

## First Commands

```bash
topclaw status
topclaw agent -m "Hello!"
topclaw gateway
```

## Documentation

- Start with the docs hub: [`docs/README.md`](docs/README.md)
- Install and bootstrap guides: [`docs/getting-started/README.md`](docs/getting-started/README.md)
- CLI, config, providers, and channels: [`docs/reference/README.md`](docs/reference/README.md)
- Operations and troubleshooting: [`docs/operations/README.md`](docs/operations/README.md)

## Other Platforms

- Windows: use `.\bootstrap.ps1`
- Lower-resource machines: run `./bootstrap.sh --prefer-prebuilt`
