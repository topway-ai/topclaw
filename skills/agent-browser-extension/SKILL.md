---
name: agent-browser-extension
description: Enable browser automation as an explicitly reviewed extension when the operator asks for interactive web navigation, page extraction, or DOM-level automation that cannot be handled by the low-risk web search path. Use this skill only when browser automation is already enabled in TopClaw config and the request has passed external security policy.
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
4. No OS-level Accessibility or unrestricted GUI automation has been granted unless the machine is dedicated to that purpose.
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
