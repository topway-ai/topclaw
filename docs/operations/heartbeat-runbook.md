# Heartbeat Runbook

Use this guide when you want TopClaw's daemon heartbeat to behave more like periodic follow-through than a blind timer.

## What Changed

Heartbeat tasks are now stateful:

- plain `- task` lines still work
- tasks keep run history in `state/heartbeat_state.json`
- each task gets a cooldown instead of re-running blindly every tick
- repeated failures back off automatically
- `max_runs=1` style tasks stop after they complete enough times
- each tick executes only the highest-priority due tasks instead of everything at once

## HEARTBEAT.md Syntax

Basic task:

```md
- Review my calendar
```

Task with metadata:

```md
- [every=4h] [priority=2] Review my calendar for the next 24 hours
- [every=1d] Check active repos for stale branches
- [every=30m] [max_runs=1] Remind me to finish onboarding notes
```

Supported metadata:

- `every=<duration>` or `cooldown=<duration>`
- `priority=<integer>`
- `max_runs=<integer>`

Duration examples:

- `30m`
- `4h`
- `1d`

## Operational Behavior

- If `HEARTBEAT.md` has tasks, TopClaw schedules those.
- If `HEARTBEAT.md` has no task bullets, TopClaw falls back to `heartbeat.message` when configured.
- New tasks are due immediately.
- Successful runs schedule the next due time from the task cooldown.
- Failed runs retry sooner at first, then back off.
- Removed tasks simply stop being selected; old state entries may remain in the state file as history.

## Files

- Task source: `<workspace>/HEARTBEAT.md`
- Task state: `<workspace>/state/heartbeat_state.json`
- Daemon health snapshot: `~/.topclaw/daemon_state.json`

## Recommended Practice

- Keep heartbeat tasks small and concrete.
- Prefer 3-10 durable tasks, not a giant wish list.
- Use `priority=2` or higher only for work you truly want first.
- Use `max_runs=1` for one-off nudges or migration reminders.
- Do not put destructive shell commands directly into heartbeat prompts.

## Smoke Check

1. Start the daemon.
2. Add one test task in `HEARTBEAT.md`.
3. Wait for one heartbeat interval.
4. Confirm `state/heartbeat_state.json` now contains `last_run_at`, `next_due_at`, and counters.
5. Confirm the same task does not immediately re-run on the next tick unless it is due again.

## Rollback

If the new scheduler cadence is not what you want:

1. Stop the daemon.
2. Simplify `HEARTBEAT.md` back to plain `- task` lines.
3. Delete `<workspace>/state/heartbeat_state.json` if you want to reset heartbeat memory.
4. Restart the daemon.
