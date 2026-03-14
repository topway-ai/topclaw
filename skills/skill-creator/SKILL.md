---
name: skill-creator
description: Guide for creating effective skills. Use when users want to create a new skill, redesign an existing skill, package a skill, validate a skill bundle, or add reusable scripts, references, and assets that make an agent more capable for a recurring task.
license: Complete terms in LICENSE.txt
---

# Skill Creator

This skill helps build high-quality skill bundles that another agent can use effectively.

It is designed for real skill development work, not just drafting a single `SKILL.md`. Use it to:

- understand the target workflow with concrete examples
- decide what belongs in `SKILL.md` versus bundled resources
- initialize a skill skeleton
- add reusable scripts, references, and assets
- validate and package the skill for distribution
- iterate after real usage

## About Skills

Skills are modular, self-contained packages that extend an agent with procedural knowledge, reusable tooling, and domain-specific guidance.

A good skill should reduce repeated work. It should not merely restate what a capable agent already knows.

In TopClaw, a skill you create with this workflow should behave like a plugin:

- create it under the active workspace `skills/` directory
- it appears as `self-added` in `topclaw skills list`
- users can disable it with `topclaw skills disable <name>`
- users can re-enable it with `topclaw skills enable <name>`
- users can remove it with `topclaw skills remove <name>`

Do not invent a separate storage location or management flow for generated skills unless the user explicitly asks for one.

### What a Skill Can Provide

1. Specialized workflows for a recurring task.
2. Tooling guidance for APIs, file formats, or domain-specific systems.
3. Reusable scripts for deterministic or repetitive actions.
4. Reference material that should be read only when needed.
5. Assets or templates that should be copied or used in final output.

## Core Principles

### Keep Context Lean

The context window is shared with everything else the agent must do. Keep `SKILL.md` lean and move detailed material into `references/` when possible.

Only include content that gives another agent something non-obvious, durable, or operationally useful.

### Match the Right Degree of Structure

Choose the least rigid approach that still keeps the task reliable:

- High freedom: plain guidance when multiple solutions are valid.
- Medium freedom: pseudocode or helper scripts when there is a preferred pattern.
- Low freedom: deterministic scripts or exact templates when consistency matters.

### Design for Progressive Disclosure

Organize the skill so an agent can discover only the information it needs:

1. Frontmatter decides when the skill should trigger.
2. `SKILL.md` explains the core workflow.
3. `references/`, `scripts/`, and `assets/` hold deeper material and reusable resources.

If a skill grows large, keep `SKILL.md` as the navigation layer and move details into bundled files.

## Anatomy of a Skill

```text
skill-name/
├── SKILL.md
├── scripts/      # executable helpers
├── references/   # docs loaded only when needed
└── assets/       # templates, boilerplate, non-context resources
```

### `SKILL.md`

`SKILL.md` must contain:

- YAML frontmatter with `name` and `description`
- the core workflow and operating guidance
- explicit references to bundled resources when those resources matter

The `description` is the primary trigger signal. Put the “when to use this skill” guidance there, not buried in the body.

### `scripts/`

Use `scripts/` for deterministic or repeated work. Typical uses:

- generating a new skill skeleton
- validating frontmatter or packaging structure
- converting or transforming files repeatedly
- enforcing a format that should not be rewritten each time

This built-in skill includes:

- `scripts/init_skill.py`
- `scripts/quick_validate.py`
- `scripts/package_skill.py`

### `references/`

Use `references/` for detailed patterns that would clutter `SKILL.md`.

This built-in skill includes:

- `references/workflows.md` for workflow structuring patterns
- `references/output-patterns.md` for output-format guidance

## Creation Workflow

Follow this sequence unless there is a good reason to skip a step.

1. Understand the skill with concrete examples.
2. Plan the reusable resources.
3. Initialize the skill.
4. Edit the skill contents.
5. Validate and package the skill.
6. Iterate after real usage.

## Step 1: Understand the Skill

Start by understanding what the skill must enable another agent to do.

Prefer concrete examples over abstract summaries. Useful questions include:

- What recurring task should this skill make easier?
- What user requests should trigger it?
- What outputs should it produce?
- What failure modes or edge cases matter?
- Which parts of the workflow are repetitive enough to script?

Do not overwhelm the user with too many questions at once. Ask the highest-signal questions first.

## Step 2: Plan the Reusable Resources

For each example workflow, ask:

1. What would an agent have to rediscover every time?
2. What repeated code should become a script?
3. What detailed guidance belongs in `references/` instead of `SKILL.md`?
4. What templates or starter files belong in `assets/`?

Use these rules:

- Put durable procedure in `SKILL.md`.
- Put exact repeated logic in `scripts/`.
- Put detailed supporting knowledge in `references/`.
- Put boilerplate or deliverable resources in `assets/`.

If the skill supports multiple variants or frameworks, keep variant selection in `SKILL.md` and move variant-specific detail into separate reference files.

## Step 3: Initialize the Skill

When creating a new skill from scratch, use the initializer script:

```bash
python scripts/init_skill.py <skill-name> --path <output-directory>
```

The initializer:

- creates a new skill directory
- writes a starter `SKILL.md`
- creates example `scripts/`, `references/`, and `assets/` directories
- adds placeholder files that can be customized or deleted

Use this instead of hand-creating the skeleton unless there is a good reason not to.

When initializing into a live TopClaw workspace, prefer the workspace `skills/` directory so the generated skill immediately follows the normal self-added plugin policy.

## Step 4: Edit the Skill

When editing the skill, remember that it is written for another agent, not for the current user.

Add information that makes future execution materially better:

- procedural knowledge
- project-specific or domain-specific constraints
- reusable examples
- references to bundled scripts and docs

### Learn Proven Patterns

Read the bundled references when they apply:

- For sequential or branching workflows: `references/workflows.md`
- For templates, examples, or output-shaping: `references/output-patterns.md`

### Frontmatter Rules

Use:

- `name`
- `description`

Keep `name` stable and machine-friendly.

Make `description` explicit about both:

- what the skill does
- when it should trigger

The description should be slightly proactive so the model does not under-trigger the skill.

### Body Rules

Write in imperative form. Tell the next agent what to do.

Good body content includes:

- a quick workflow overview
- decision points
- references to scripts and docs
- concrete examples
- quality or validation expectations

Bad body content includes:

- lengthy redundant explanation
- setup clutter unrelated to execution
- auxiliary docs that belong in `references/`

## Step 5: Validate and Package

After the skill is ready, validate and package it.

### Quick Validation

Run:

```bash
python scripts/quick_validate.py <path/to/skill>
```

This checks:

- `SKILL.md` existence
- frontmatter format
- required fields
- naming conventions
- description validity

### Packaging

Run:

```bash
python scripts/package_skill.py <path/to/skill-folder>
```

Optional output directory:

```bash
python scripts/package_skill.py <path/to/skill-folder> ./dist
```

Packaging will:

1. validate the skill
2. create a distributable `.skill` archive
3. preserve the skill directory structure

If validation fails, fix the issues first and rerun packaging.

## Step 6: Iterate

A skill is rarely correct on the first try. After real usage:

1. inspect where the agent struggled
2. notice repeated ad hoc work that should become a script or reference
3. update `SKILL.md` or bundled resources
4. validate again

If multiple test runs all create the same helper code, that is a strong sign the skill should bundle that helper permanently.

## Output Expectations

When using this skill for a user request:

- identify the current stage of the skill-development workflow
- move the user forward from that stage instead of restarting from scratch
- recommend scripts and references only when they materially reduce repeated work
- keep the proposed skill structure simple and auditable

If the user already has a skill draft, skip directly to critique, restructuring, resource planning, validation, or packaging as appropriate.
