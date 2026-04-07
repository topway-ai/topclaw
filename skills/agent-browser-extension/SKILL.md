---
name: agent-browser-extension
description: "Trigger only when the user explicitly needs interactive browser navigation, DOM-level page extraction, or in-page automation that cannot be served by safe-web-search or web_fetch. Before using, verify all of: browser.enabled is true, browser.backend is set to agent_browser, the target host is in browser.allowed_domains, and external policy permits the action. If any check fails, refuse and explain the gate — do not expand allowed domains from inside the session. Keep sessions task-scoped and short-lived. Never use persistent profiles, saved cookies, or shared browser state. Stop immediately if the task would require passwords, MFA, wallet actions, payment flows, or sensitive personal data entry. Always prefer safe-web-search first. Read skills/agent-browser-extension/SKILL.md for the required operator checks, installation workflow, and output expectations."
---

# Agent Browser Extension

This is an optional extension skill for interactive browser automation.

It is intentionally not preloaded by default. Install it only after running the
skill vetter and confirming that browser automation is acceptable for the target
machine and workflow.

## Intended use

Use this skill only when:

- the user explicitly needs interactive browser automation
- low-risk search or fetch tools are insufficient
- the TopClaw `browser` tool is already enabled by operator configuration
- allowed domains and approval rules are already defined externally

## Required operator checks

Before using this skill, confirm all of the following:

1. `browser.enabled = true` is explicitly set by the operator.
2. `browser.backend = "agent_browser"` is selected intentionally.
3. `browser.allowed_domains` is narrow and task-specific.
4. No broad system-control permissions have been granted unless the machine is dedicated to this workflow.
5. Login flows, secrets, payment flows, and identity-provider pages are blocked unless external policy explicitly allows them.

## Guardrails

- Prefer `safe-web-search` or `web_fetch` before browser automation.
- Keep sessions task-scoped and short-lived.
- Do not use persistent profiles, saved cookies, or shared browser state unless the operator explicitly approved them.
- Do not bypass browser sandboxing or launch with unsafe flags.
- Stop immediately if the task would require passwords, MFA, wallet actions, or sensitive personal data entry.
- Do not expand allowed domains inside the session. That remains an operator decision.

## Installation workflow

Run the audit first:

```bash
topclaw skills vet ./skills/agent-browser-extension --json
```

Then install from the local reviewed source:

```bash
topclaw skills install ./skills/agent-browser-extension
```

## Output expectations

- State why browser automation is necessary.
- Name the exact allowed domains involved.
- Keep interaction plans minimal and reversible.
- Report what was observed, not just that the browser step succeeded.
