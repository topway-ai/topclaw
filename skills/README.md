# Preloaded Skills

This directory contains preloaded, transparent skill bundles that TopClaw copies into each workspace's `skills/` directory during initialization.

Current preloaded skills:

- `find-skills` (source: https://skills.sh/vercel-labs/skills/find-skills)
- `skill-creator` (source: https://skills.sh/anthropics/skills/skill-creator)
- `local-file-analyzer` (source: https://github.com/jackfly8/TopClaw/tree/main/skills/local-file-analyzer)
- `workspace-search` (source: https://github.com/jackfly8/TopClaw/tree/main/skills/workspace-search)
- `code-explainer` (source: https://github.com/jackfly8/TopClaw/tree/main/skills/code-explainer)
- `change-summary` (source: https://github.com/jackfly8/TopClaw/tree/main/skills/change-summary)
- `safe-web-search` (source: https://github.com/jackfly8/TopClaw/tree/main/skills/safe-web-search)

These files are committed for reviewability so users can audit exactly what ships by default.

Optional audited extension skills in this repository are not preloaded. Install them only after vetting the local source.

- `agent-browser-extension`
  - Review: `topclaw skills vet ./skills/agent-browser-extension --json`
  - Install: `topclaw skills install ./skills/agent-browser-extension`
