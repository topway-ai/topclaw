---
name: desktop-computer-use
description: "Trigger only when the user explicitly asks TopClaw to drive real desktop applications or OS windows (not just browse the web) on macOS, Windows, or Linux. If browser.enabled is false or browser.backend is not 'computer_use', do NOT refuse — instead call config_patch to set browser.enabled=true and browser.backend='computer_use' (both are approval-gated). If the sidecar endpoint is unreachable, call computer_use_sidecar_start (approval-gated) to spawn the built-in sidecar. Only refuse if the user denies approval. For each task: list candidate windows, focus the intended one, capture the screen, plan the smallest reversible step, execute one mouse or keyboard action, then re-capture and verify. Stop immediately on password, MFA, wallet, payment, privileged admin, system-settings, or unbounded file-deletion prompts. Treat app_launch and app_terminate as higher-risk than focus or capture. Prefer safe-web-search, web_fetch, or agent-browser-extension before reaching for OS-level control. Read skills/desktop-computer-use/SKILL.md for the full action surface and the one-tap enablement flow."
---

# Desktop Computer Use

This is an optional extension skill for OS-level desktop automation.

It is intentionally not preloaded by default. Install it only after the
operator confirms that desktop automation is acceptable for the machine,
session, and data involved.

## Intended use

Use this skill only when the user explicitly asks TopClaw to operate desktop
software, not just browse the web, and the machine is isolated enough that
accidental clicks or typing will not cause unacceptable harm.

If preconditions are missing, **do not refuse**. Propose the enablement flow
below — each step is approval-gated, so the user has final say, but a single
approved sequence gets the skill working end-to-end.

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

## Enablement flow (one tap per gate)

When a precondition is not yet satisfied, call these tools in order. Each one
emits an approval prompt the user taps Approve on — nothing runs unattended.

1. `config_patch` with `{path: "browser.enabled", value: true}` — flips the
   top-level browser gate.
2. `config_patch` with `{path: "browser.backend", value: "computer_use"}` —
   routes the browser tool through the computer-use sidecar.
3. `computer_use_sidecar_start` with `{bind: "127.0.0.1:8787"}` — spawns the
   built-in sidecar (Linux only at present; see below). Idempotent: if a
   healthy sidecar is already listening, the tool returns success without
   spawning a duplicate.
4. Optionally `config_patch` with
   `{path: "browser.computer_use.window_allowlist", value: [...]}` to narrow
   the allowed window titles for the task at hand.

If the user denies any approval, stop and explain which capability is blocked.
Do not attempt to work around a denial.

## Required operator awareness

Before proposing the flow, confirm the user understands:

- The current machine and desktop session do not contain unattended payment,
  wallet, privileged admin, or private personal workflows that must never be
  touched.
- OS-level automation can click the wrong thing if the desktop changes
  unexpectedly; the agent will re-capture and verify after each action.
- The built-in sidecar is Linux-only (shells out to `xdotool`, `wmctrl`,
  `scrot`/`gnome-screenshot`, `xdg-open`, `pkill`). On macOS or Windows, the
  user must install and run a protocol-compatible sidecar (see
  `docs/computer-use-sidecar-protocol.md`) before step 3 will succeed.

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
