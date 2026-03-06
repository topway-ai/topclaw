# TopClaw

TopClaw is a Rust-based AI agent runtime.

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

topclaw onboard --interactive
topclaw status
topclaw agent -m "Hello!"
```

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

topclaw onboard --interactive
topclaw status
topclaw agent -m "Hello!"
```

Fast path if you already have an API key:

```bash
topclaw onboard --api-key "sk-..." --provider openrouter
```

Start the local gateway after onboarding:

```bash
topclaw gateway
```

### Other Platforms

- Windows: run `.\bootstrap.ps1`
- Lower-resource machines: `./bootstrap.sh --prefer-prebuilt`
