---
name: change-summary
description: "Trigger when the user asks what changed, what to review, or what shipped, or wants a diff, commit-range, or release-delta summary. Inspect the actual diff, commit range, or status view (never invent history), group changes by behavior or subsystem, separate user-facing impact from internal cleanup, and flag risky edits, migrations, or missing validation. Cite commit SHAs or file paths. Exclude pure formatting noise unless it affects review. Do not claim tests passed unless you actually verified them. Note uncertainty when a change was not fully validated."
---

# Change Summary

Use this skill to explain local repository changes clearly and conservatively.

## Goals

- Identify the highest-signal behavior changes.
- Separate user-facing impact from internal cleanup.
- Make review and release notes easier to understand.

## Workflow

1. Inspect the relevant diff, commit range, or status view.
2. Group changes by behavior, subsystem, or user impact.
3. Call out risky edits, migrations, and missing validation.
4. Exclude noise such as pure formatting unless it affects review.
5. Note uncertainty when a change was not fully validated.

## Guardrails

- Do not claim tests passed unless you verified them.
- Do not invent intent; infer only from the diff and commit context.
- Keep summaries compact and review-focused.

## Output

- Lead with the main change.
- List the highest-impact file groups or behaviors.
- Mention validation status and remaining risks.
