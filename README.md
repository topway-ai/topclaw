<p align="center">
  <img src="topclaw.png" alt="TopClaw" width="200" />
</p>

<h1 align="center">TopClaw 🦀</h1>

<p align="center">
  <strong>Zero overhead. Zero compromise. 100% Rust. 100% Agnostic.</strong><br>
  ⚡️ <strong>Runs on $10 hardware with <5MB RAM: That's 99% less memory than OpenClaw and 98% cheaper than a Mac mini!</strong>
</p>

<p align="center">
  <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
  <a href="NOTICE"><img src="https://img.shields.io/github/contributors/jackfly8/TopClaw?color=green" alt="Contributors" /></a>
  <a href="https://buymeacoffee.com/argenistherose"><img src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-Donate-yellow.svg?style=flat&logo=buy-me-a-coffee" alt="Buy Me a Coffee" /></a>
  <a href="https://x.com/topclawlabs?s=21"><img src="https://img.shields.io/badge/X-%40topclawlabs-000000?style=flat&logo=x&logoColor=white" alt="X: @topclawlabs" /></a>
  <a href="https://topclawlabs.cn/group.jpg"><img src="https://img.shields.io/badge/WeChat-Group-B7D7A8?logo=wechat&logoColor=white" alt="WeChat Group" /></a>
  <a href="https://www.xiaohongshu.com/user/profile/67cbfc43000000000d008307?xsec_token=AB73VnYnGNx5y36EtnnZfGmAmS-6Wzv8WMuGpfwfkg6Yc%3D&xsec_source=pc_search"><img src="https://img.shields.io/badge/Xiaohongshu-Official-FF2442?style=flat" alt="Xiaohongshu: Official" /></a>
  <a href="https://t.me/topclawlabs"><img src="https://img.shields.io/badge/Telegram-%40topclawlabs-26A5E4?style=flat&logo=telegram&logoColor=white" alt="Telegram: @topclawlabs" /></a>
  <a href="https://www.facebook.com/groups/topclaw"><img src="https://img.shields.io/badge/Facebook-Group-1877F2?style=flat&logo=facebook&logoColor=white" alt="Facebook Group" /></a>
  <a href="https://www.reddit.com/r/topclawlabs/"><img src="https://img.shields.io/badge/Reddit-r%2Ftopclawlabs-FF4500?style=flat&logo=reddit&logoColor=white" alt="Reddit: r/topclawlabs" /></a>
</p>
<p align="center">
Built by students and members of the Harvard, MIT, and Sundai.Club communities.
</p>

<p align="center">
  🌐 <strong>Languages:</strong> <a href="README.md">English</a> · <a href="docs/i18n/zh-CN/README.md">简体中文</a> · <a href="docs/i18n/ja/README.md">日本語</a> · <a href="docs/i18n/ru/README.md">Русский</a> · <a href="docs/i18n/fr/README.md">Français</a> · <a href="docs/i18n/vi/README.md">Tiếng Việt</a> · <a href="docs/i18n/el/README.md">Ελληνικά</a>
</p>

<p align="center">
  <a href="#quick-start">Getting Started</a> |
  <a href="bootstrap.sh">One-Click Setup</a> |
  <a href="docs/README.md">Docs Hub</a> |
  <a href="docs/SUMMARY.md">Docs TOC</a>
</p>

<p align="center">
  <strong>Quick Routes:</strong>
  <a href="docs/reference/README.md">Reference</a> ·
  <a href="docs/operations/README.md">Operations</a> ·
  <a href="docs/troubleshooting.md">Troubleshoot</a> ·
  <a href="docs/security/README.md">Security</a> ·
  <a href="docs/hardware/README.md">Hardware</a> ·
  <a href="docs/contributing/README.md">Contribute</a>
</p>

<p align="center">
  <strong>Fast, small, and fully autonomous AI assistant infrastructure</strong><br />
  Deploy anywhere. Swap anything.
</p>

<p align="center">
  TopClaw is the <strong>runtime operating system</strong> for agentic workflows — infrastructure that abstracts models, tools, memory, and execution so agents can be built once and run anywhere.
</p>

<p align="center"><code>Trait-driven architecture · secure-by-default runtime · provider/channel/tool swappable · pluggable everything</code></p>

### 📢 Announcements

Use this board for important notices (breaking changes, security advisories, maintenance windows, and release blockers).

| Date (UTC) | Level       | Notice                                                                                                                                                                                                                                                                                                                                                 | Action                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------- | ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-02-19 | _Critical_  | We are **not affiliated** with `openagen/topclaw`, `topclaw.org` or `topclaw.net`. The `topclaw.org` and `topclaw.net` domains currently points to the `openagen/topclaw` fork, and that domain/repository are impersonating our official website/project.                                                                                       | Do not trust information, binaries, fundraising, or announcements from those sources. Use only [this repository](https://github.com/jackfly8/TopClaw) and our verified social accounts.                                                                                                                                                                                                                                                                                                                                                                                                                            |
| 2026-02-21 | _Important_ | Our official website is now live: [topclawlabs.ai](https://topclawlabs.ai). Thanks for your patience while we prepared the launch. We are still seeing impersonation attempts, so do **not** join any investment or fundraising activity claiming the TopClaw name unless it is published through our official channels.                            | Use [this repository](https://github.com/jackfly8/TopClaw) as the single source of truth. Follow [X (@topclawlabs)](https://x.com/topclawlabs?s=21), [Telegram (@topclawlabs)](https://t.me/topclawlabs), [Facebook (Group)](https://www.facebook.com/groups/topclaw), [Reddit (r/topclawlabs)](https://www.reddit.com/r/topclawlabs/), and [Xiaohongshu](https://www.xiaohongshu.com/user/profile/67cbfc43000000000d008307?xsec_token=AB73VnYnGNx5y36EtnnZfGmAmS-6Wzv8WMuGpfwfkg6Yc%3D&xsec_source=pc_search) for official updates. |
| 2026-02-19 | _Important_ | Anthropic updated the Authentication and Credential Use terms on 2026-02-19. Claude Code OAuth tokens (Free, Pro, Max) are intended exclusively for Claude Code and Claude.ai; using OAuth tokens from Claude Free/Pro/Max in any other product, tool, or service (including Agent SDK) is not permitted and may violate the Consumer Terms of Service. | Please temporarily avoid Claude Code OAuth integrations to prevent potential loss. Original clause: [Authentication and Credential Use](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use).                                                                                                                                                                                                                                                                                                                                                                                    |

### ✨ Features

- 🏎️ **Lean Runtime by Default:** Common CLI and status workflows run in a few-megabyte memory envelope on release builds.
- 💰 **Cost-Efficient Deployment:** Designed for low-cost boards and small cloud instances without heavyweight runtime dependencies.
- ⚡ **Fast Cold Starts:** Single-binary Rust runtime keeps command and daemon startup near-instant for daily operations.
- 🌍 **Portable Architecture:** One binary-first workflow across ARM, x86, and RISC-V with swappable providers/channels/tools.
- 🔍 **Research Phase:** Proactive information gathering through tools before response generation — reduces hallucinations by fact-checking first.

### Why teams pick TopClaw

- **Lean by default:** small Rust binary, fast startup, low memory footprint.
- **Secure by design:** pairing, strict sandboxing, explicit allowlists, workspace scoping.
- **Fully swappable:** core systems are traits (providers, channels, tools, memory, tunnels).
- **No lock-in:** OpenAI-compatible provider support + pluggable custom endpoints.

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

# First-time setup
topclaw onboard --interactive

# Fast path if you already have an API key
topclaw onboard --api-key "sk-..." --provider openrouter

# Verify the install
topclaw status

# Try one message
topclaw agent -m "Hello!"
```

If you want the browser UI after onboarding:

```bash
topclaw gateway
```

Open the local URL printed in startup logs.

### Other Platforms

- macOS/Linuxbrew: `brew install topclaw`
- Windows: clone the repo and run `.\bootstrap.ps1`
- Docker and advanced bootstrap flows: [docs/one-click-bootstrap.md](docs/one-click-bootstrap.md)

### Notes

- Source builds typically need about 2GB RAM and about 6GB disk.
- If you are on a smaller machine, try `./bootstrap.sh --prefer-prebuilt`.
- The main first-run commands are `topclaw onboard`, `topclaw status`, `topclaw agent`, and `topclaw gateway`.

### Installation Docs (Canonical Source)

Use repository docs as the source of truth for install/setup instructions:

- [README Quick Start](#quick-start)
- [docs/one-click-bootstrap.md](docs/one-click-bootstrap.md)
- [docs/getting-started/README.md](docs/getting-started/README.md)

Issue comments can provide context, but they are not canonical installation documentation.
## Benchmark Snapshot (TopClaw vs OpenClaw, Reproducible)

Local machine quick benchmark (macOS arm64, Feb 2026) normalized for 0.8GHz edge hardware.

|                           | OpenClaw      | NanoBot        | PicoClaw        | TopClaw 🦀          |
| ------------------------- | ------------- | -------------- | --------------- | -------------------- |
| **Language**              | TypeScript    | Python         | Go              | **Rust**             |
| **RAM**                   | > 1GB         | > 100MB        | < 10MB          | **< 5MB**            |
| **Startup (0.8GHz core)** | > 500s        | > 30s          | < 1s            | **< 10ms**           |
| **Binary Size**           | ~28MB (dist)  | N/A (Scripts)  | ~8MB            | **~8.8 MB**          |
| **Cost**                  | Mac Mini $599 | Linux SBC ~$50 | Linux Board $10 | **Any hardware $10** |

> Notes: TopClaw results are measured on release builds using `/usr/bin/time -l`. OpenClaw requires Node.js runtime (typically ~390MB additional memory overhead), while NanoBot requires Python runtime. PicoClaw and TopClaw are static binaries. The RAM figures above are runtime memory; build-time compilation requirements are higher.

<p align="center">
  <img src="zero-claw.jpeg" alt="TopClaw vs OpenClaw Comparison" width="800" />
</p>

---

For full documentation, see [`docs/README.md`](docs/README.md) | [`docs/SUMMARY.md`](docs/SUMMARY.md)

## ⚠️ Official Repository & Impersonation Warning

**This is the only official TopClaw repository:**

> https://github.com/jackfly8/TopClaw

Any other repository, organization, domain, or package claiming to be "TopClaw" or implying affiliation with TopClaw Labs is **unauthorized and not affiliated with this project**. Known unauthorized forks will be listed in [TRADEMARK.md](TRADEMARK.md).

If you encounter impersonation or trademark misuse, please [open an issue](https://github.com/jackfly8/TopClaw/issues).

---

## License

TopClaw is dual-licensed for maximum openness and contributor protection:

| License | Use case |
|---|---|
| [MIT](LICENSE-MIT) | Open-source, research, academic, personal use |
| [Apache 2.0](LICENSE-APACHE) | Patent protection, institutional, commercial deployment |

You may choose either license. **Contributors automatically grant rights under both** — see [CLA.md](CLA.md) for the full contributor agreement.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [CLA.md](CLA.md). Implement a trait, submit a PR.

---

**TopClaw** — Zero overhead. Zero compromise. Deploy anywhere. Swap anything. 🦀

## Star History

<p align="center">
  <a href="https://www.star-history.com/#jackfly8/TopClaw&type=date&legend=top-left">
    <picture>
     <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=jackfly8/TopClaw&type=date&theme=dark&legend=top-left" />
     <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=jackfly8/TopClaw&type=date&legend=top-left" />
     <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=jackfly8/TopClaw&type=date&legend=top-left" />
    </picture>
  </a>
</p>
