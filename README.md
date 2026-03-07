# TopClaw

TopClaw is a Rust-based AI agent runtime for local and remote AI workflows.

## Quick Start

### Ubuntu

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev ca-certificates curl git

curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"

git clone https://github.com/jackfly8/TopClaw.git
cd TopClaw
./bootstrap.sh
topclaw status
topclaw agent -m "Hello!"
```

`./bootstrap.sh` installs missing prerequisites, builds TopClaw, and starts onboarding automatically.

### macOS (Apple Silicon)

Install Apple developer tools first:

```bash
xcode-select --install
```

Install Rust:

```bash
curl https://sh.rustup.rs -sSf | sh
source "$HOME/.cargo/env"
```

Clone and bootstrap TopClaw:

```bash
git clone https://github.com/jackfly8/TopClaw.git
cd TopClaw
./bootstrap.sh
topclaw status
topclaw agent -m "Hello!"
```

`./bootstrap.sh` installs missing prerequisites, builds TopClaw, and starts onboarding automatically.

## What Bootstrap Does

Running `./bootstrap.sh` with no flags will:

1. install missing system dependencies when possible
2. install Rust if it is not already present
3. build and install `topclaw`
4. start the onboarding wizard

During onboarding, the default path is now:

- choose your AI provider
- enter the provider API key if needed
- choose a channel such as Telegram or Discord
- enter the channel token and allowed user info

Everything else can be changed later in `config.toml`.

## Fast Path

If you already have an API key and want a minimal setup:

```bash
topclaw onboard --api-key "sk-..." --provider openrouter
```

## First Commands

After onboarding, these are the most useful first commands:

```bash
topclaw status
topclaw agent -m "Hello!"
topclaw gateway
```

## Documentation Map

- Getting started: [`docs/getting-started/README.md`](docs/getting-started/README.md)
- Commands and config: [`docs/reference/README.md`](docs/reference/README.md)
- Operations and troubleshooting: [`docs/operations/README.md`](docs/operations/README.md)
- Full docs hub: [`docs/README.md`](docs/README.md)

### Other Platforms

- Windows: run `.\bootstrap.ps1`
- Lower-resource machines: `./bootstrap.sh --prefer-prebuilt`
