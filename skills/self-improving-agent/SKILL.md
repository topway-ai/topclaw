---
name: self-improving-agent
description: "Trigger when a non-trivial command, tool, test, or workflow fails; the user corrects an assumption, fact, or interpretation; an external API/SDK/integration behaves unexpectedly; the user requests a missing capability; or a recurring better approach is worth preserving. Append a structured entry to one of `.learnings/LEARNINGS.md` (durable lessons, corrections, best practices), `.learnings/ERRORS.md` (failures and workarounds), or `.learnings/FEATURE_REQUESTS.md` (missing capabilities) using the `TYPE-YYYYMMDD-NNN` ID format (LRN, ERR, FEAT). Keep entries specific and actionable, link affected files or commands, and update existing entries instead of duplicating recurring patterns. Never store secrets, raw tokens, cookies, private URLs, or unredacted personal data — replace sensitive values with `[REDACTED_TOKEN]` or `[REDACTED_EMAIL]` placeholders. Do not log trivial one-off noise. Read skills/self-improving-agent/SKILL.md for entry templates, routing rules, and the resolution workflow."
---

# Self-Improving Agent

Use this skill to record durable learnings in the current workspace without changing default runtime behavior for every session.

This skill is intentionally prompt-only so it remains installable under TopClaw's low-risk skill policy.

## When To Use

Use this skill when one of these happens:

- A command, tool, test, or workflow fails in a non-trivial way.
- The user corrects a mistaken assumption, fact, or interpretation.
- An external API, SDK, service, or integration behaves differently than expected.
- The user asks for a capability the current project or agent setup does not provide.
- You discover a better repeatable approach worth preserving for later tasks.
- You are starting a large task and want to review prior workspace learnings first.

Do not log trivial one-off noise. Prefer entries that could change future behavior, reduce repeated mistakes, or help another agent continue the work.

## Files

Store entries under the current workspace root:

```text
.learnings/
├── LEARNINGS.md
├── ERRORS.md
└── FEATURE_REQUESTS.md
```

If `.learnings/` or one of these files does not exist, create it only when you have a concrete entry to add.

## Routing Rules

| Situation | File |
|-----------|------|
| Durable lesson, correction, best practice, project convention | `.learnings/LEARNINGS.md` |
| Failed command, runtime error, bad tool behavior, flaky integration | `.learnings/ERRORS.md` |
| Requested capability that does not exist yet | `.learnings/FEATURE_REQUESTS.md` |

If an item fits more than one category, prefer the file that best matches the next action:

- fix or workaround work: `ERRORS.md`
- future implementation work: `FEATURE_REQUESTS.md`
- long-term guidance: `LEARNINGS.md`

## Operating Rules

- Keep entries specific and actionable.
- Link to affected files, commands, or subsystems when known.
- Reuse or update an existing entry instead of creating duplicates when the pattern is recurring.
- Mark status changes when an issue is fixed or a request is completed.
- If a learning should influence repository policy or shared docs, propose that follow-up explicitly instead of silently editing protected guidance files.
- Never store secrets, credentials, raw tokens, cookies, private URLs, or unredacted personal data in `.learnings/`.
- Redact sensitive values from command output before writing excerpts. Replace them with placeholders such as `[REDACTED_TOKEN]` or `[REDACTED_EMAIL]`.
- If an error log or environment detail might contain sensitive material, summarize it instead of copying it verbatim.

## Entry Formats

### Learning Entry

Append to `.learnings/LEARNINGS.md`:

```markdown
## [LRN-YYYYMMDD-XXX] category

**Logged**: ISO-8601 timestamp
**Priority**: low | medium | high | critical
**Status**: pending
**Area**: frontend | backend | infra | tests | docs | config | runtime

### Summary
One-line description of the learning

### Details
What happened, what was learned, and why it matters

### Suggested Action
Specific follow-up or behavior change

### Metadata
- Source: conversation | error | user_feedback | investigation
- Related Files: path/to/file.ext
- Tags: tag1, tag2
- See Also: LRN-YYYYMMDD-XXX

---
```

Recommended categories:

- `correction`
- `knowledge_gap`
- `best_practice`
- `workflow`
- `integration`
- `testing`

### Error Entry

Append to `.learnings/ERRORS.md`:

```markdown
## [ERR-YYYYMMDD-XXX] command_or_component

**Logged**: ISO-8601 timestamp
**Priority**: medium | high | critical
**Status**: pending
**Area**: frontend | backend | infra | tests | docs | config | runtime

### Summary
Brief description of the failure

### Error
Indented literal error output or the most relevant excerpt

### Context
- Command or operation attempted
- Inputs, flags, or parameters
- Environment details if relevant

### Suggested Fix
Most likely workaround, next diagnostic step, or repair

### Metadata
- Reproducible: yes | no | unknown
- Related Files: path/to/file.ext
- See Also: ERR-YYYYMMDD-XXX

---
```

### Feature Request Entry

Append to `.learnings/FEATURE_REQUESTS.md`:

```markdown
## [FEAT-YYYYMMDD-XXX] capability_name

**Logged**: ISO-8601 timestamp
**Priority**: low | medium | high
**Status**: pending
**Area**: frontend | backend | infra | tests | docs | config | runtime

### Requested Capability
What the user wants to do

### User Context
Why it matters for the current workflow

### Suggested Implementation
Smallest viable implementation path

### Metadata
- Frequency: first_time | recurring
- Related Files: path/to/file.ext
- Related Features: existing_feature_name

---
```

## ID Format

Use `TYPE-YYYYMMDD-XXX`:

- `LRN` for learnings
- `ERR` for errors
- `FEAT` for feature requests
- `YYYYMMDD` for the current date
- `XXX` for a short sequence such as `001`

Examples:

- `LRN-20260307-001`
- `ERR-20260307-001`
- `FEAT-20260307-001`

## Resolution Workflow

When an item is addressed:

1. Change `**Status**` from `pending` to `resolved`, `in_progress`, `wont_fix`, or `promoted`.
2. Add a short resolution block:

```markdown
### Resolution
- **Resolved**: ISO-8601 timestamp
- **Change**: commit, PR, or manual fix reference
- **Notes**: what was done
```

Use `promoted` only when the learning has been intentionally folded into lasting repository guidance or documentation with user or maintainer approval.

## Recommended Workflow

1. Review `.learnings/` before major implementation work if the workspace already uses it.
2. Add a new entry immediately after a meaningful failure, correction, or discovery.
3. Re-check for related entries before creating another item for the same pattern.
4. Surface high-value repeated items in your final summary so the operator can decide whether to promote them into code, docs, or policy.
