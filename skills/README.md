# TopClaw Skills

This directory contains the curated skill bundles that TopClaw can copy into a workspace.

On CLI onboarding, TopClaw presents these skills in one list with recommendation labels:

## Recommended Starter Skills

These are recommended by default because they stay focused on local explanation, read-only analysis, or skill authoring.

- `find-skills` — discover and install extra skills for recurring tasks
- `skill-creator` — create, validate, and package reusable skill bundles
- `local-file-analyzer` — read and summarize local files without editing them
- `workspace-search` — search code, docs, and config inside the current workspace
- `code-explainer` — explain modules, control flow, and behavior from existing code
- `change-summary` — summarize diffs, commits, and release deltas clearly
- `safe-web-search` — look up current information with low-risk web search tools

## Advanced Optional Skills

These remain opt-in because they reach outside the workspace, write durable notes, or automate external surfaces.

- `self-improving-agent` — write durable learnings and failure notes into the workspace
- `multi-search-engine` — use specific public search engines and advanced query operators
- `agent-browser-extension` — drive approved websites with interactive browser automation
- `desktop-computer-use` — control real desktop apps and windows through computer-use tooling

## Install behavior

- Onboarding installs curated skills into the workspace from their reviewed sources.
- Every listed skill remains optional during onboarding.
- Recommended starter skills are selected by default during onboarding.
- Advanced optional skills are shown unchecked until you explicitly opt in.
- Curated installs prefer a local TopClaw repo checkout when available, which keeps prebuilt binaries small while preserving reviewed skill sources.
- Curated advanced optional skills can still be installed later with `topclaw skills install <source>` after review.

These files are committed for reviewability so users can audit exactly what ships with the repository.
