# AGENTS.md — TopClaw Agent Engineering Protocol

This file extends [`CLAUDE.md`](CLAUDE.md) for non-Claude coding agents.

**All sections in `CLAUDE.md` apply to every agent.** This file adds only
the following supplementary rules.

## Action Preflight for Side Effects

Write/execute/network/config actions need an explicit self-check before
blast radius expands.

Before any write, shell command, deletion, network access, or config change:

1. State the exact goal.
2. List the exact files, commands, and endpoints involved.
3. Classify risk as `low`, `medium`, `high`, or `critical`.
4. Describe the blast radius if the action fails.
5. State whether the action is reversible.
6. Provide a rollback plan.
7. Prefer the smallest effective change.
8. Stop immediately on anomaly or unexpected output.
9. Never modify `AGENTS.md`, `CLAUDE.md`, secrets, keys, or runtime config
   unless explicitly permitted by external policy.
10. After each action, verify results before continuing.

This preflight is a required operating discipline, not a substitute for
external policy enforcement.
