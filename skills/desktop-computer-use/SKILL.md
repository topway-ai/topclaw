---
name: desktop-computer-use
description: "Trigger only when the user explicitly asks TopClaw to drive real desktop applications or OS windows (not just browse the web) on macOS, Windows, or Linux. Before using, verify all of: browser.enabled is true, browser.backend is set to computer_use, a compatible computer-use sidecar is running, browser.computer_use.endpoint points to a trusted local or explicitly approved remote sidecar, and browser.computer_use.window_allowlist is narrow and task-scoped. If any check fails, refuse and explain the gate. For each task: list candidate windows, focus the intended one, capture the screen, plan the smallest reversible step, execute one mouse or keyboard action, then re-capture and verify. Stop immediately on password, MFA, wallet, payment, privileged admin, system-settings, or unbounded file-deletion prompts. Treat app_launch and app_terminate as higher-risk than focus or capture. Prefer safe-web-search, web_fetch, or agent-browser-extension before reaching for OS-level control. Read skills/desktop-computer-use/SKILL.md for the full action surface and operator checks."
---

# Desktop Computer Use

This is an optional extension skill for OS-level desktop automation.

It is intentionally not preloaded by default. Install it only after the
operator confirms that desktop automation is acceptable for the machine,
session, and data involved.

## Intended use

Use this skill only when:

- the user explicitly asks TopClaw to operate desktop software, not just browse the web
- `browser.enabled = true` is already set
- `browser.backend = "computer_use"` is already configured intentionally
- a compatible computer-use sidecar is already running for the current OS
- the machine is isolated enough that accidental clicks or typing will not cause unacceptable harm

This skill is designed for:

- macOS
- Windows
- Linux

It assumes the sidecar exposes TopClaw's computer-use action surface through the
`browser` tool, including:

- `screen_capture`
- `mouse_move`
- `mouse_click`
- `mouse_drag`
- `key_type`
- `key_press`
- `window_list`
- `window_focus`
- `window_close`
- `app_launch`
- `app_terminate`

## Required operator checks

Before using this skill, confirm all of the following:

1. `browser.enabled = true` is explicitly set by the operator.
2. `browser.backend = "computer_use"` is selected intentionally.
3. `browser.computer_use.endpoint` points to a trusted local or explicitly approved remote sidecar.
4. `browser.computer_use.window_allowlist` is narrow and task-scoped when possible.
5. The current machine and desktop session do not contain unattended payment, wallet, privileged admin, or private personal workflows that must never be touched.
6. The user understands that OS-level automation can click the wrong thing if the desktop changes unexpectedly.

## Guardrails

- Prefer `safe-web-search`, `web_fetch`, or DOM-level browser automation before OS-level desktop control.
- Start every task by discovering the desktop state:
  - list candidate windows
  - focus the intended window
  - capture the screen
- Use the smallest reversible action possible.
- Re-capture the screen after meaningful transitions instead of assuming the UI changed as expected.
- Launch or focus one application at a time.
- Avoid broad typing into unknown focused fields.
- Stop immediately if the task would require passwords, MFA, wallet approvals, production admin panels, system settings changes, or unbounded file deletion.
- Treat app launch and termination as higher-risk actions than window focus or screen capture.
- Do not expand window scope beyond the operator-approved task.

## Operating pattern

For each task:

1. Identify the target application or window.
2. Use `window_list` or `app_launch` if needed.
3. Use `window_focus` to bring the target forward.
4. Use `screen_capture` to inspect the current UI.
5. Plan the next smallest action.
6. Execute a single mouse or keyboard step.
7. Capture again and verify outcome.
8. Repeat until the requested task is complete or uncertainty becomes too high.

If the UI is ambiguous:

- stop and ask for confirmation rather than guessing

If the focused window changes unexpectedly:

- stop, re-list windows, and re-focus intentionally

## Installation workflow

Run the audit first:

```bash
topclaw skills vet ./skills/desktop-computer-use --json
```

Then install from the local reviewed source:

```bash
topclaw skills install ./skills/desktop-computer-use
```

## Output expectations

- State why desktop computer use is necessary.
- Name the exact applications or windows involved.
- State whether the sidecar is expected to run on macOS, Windows, or Linux.
- Keep plans short, explicit, and reversible.
- Report what changed on screen after each important step.
