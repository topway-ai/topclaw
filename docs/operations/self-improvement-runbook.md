# TopClaw Self-Improvement Runbook

This runbook defines a phase-1, operator-controlled model for letting TopClaw improve a candidate copy of itself without modifying the running stable copy in place.

## Goal

Give TopClaw controlled self-iteration ability:

- inspect and diagnose its own code
- edit a candidate copy
- run validation on that candidate
- never replace the running stable copy directly
- require an external promotion step

This is intentionally conservative. It is not an auto-update or auto-deploy system.

## Core Rules

1. TopClaw may edit only `candidate`.
2. TopClaw may not directly restart, replace, or deploy `stable`.
3. Promotion must be performed by an external script or a human operator.

## Directory Model

Default base directory:

```bash
mkdir -p ~/topclaw-self/{stable,candidate,releases,logs}
```

Meaning:

- `stable`: current trusted checkout
- `candidate`: disposable writable copy for self-improvement work
- `releases`: validated snapshots produced by promotion
- `logs`: promotion logs

## Phase-1 Workflow

### 1. Seed the stable checkout

```bash
git clone https://github.com/topway-ai/TopClaw.git ~/topclaw-self/stable
```

### 2. Prepare a fresh candidate

Use the helper script:

```bash
scripts/self-improve/prepare_candidate.sh
```

This:

- ensures the base directories exist
- verifies `stable` is a git checkout
- replaces `candidate` with a fresh copy of `stable`
- creates a dedicated candidate branch when possible

### 3. Give TopClaw a narrow task

Use a prompt shaped like the template at:

- `scripts/self-improve/prompt-template.txt`

Recommended task shape:

- one bug fix
- one test gap
- one docs correction
- one small refactor

Avoid broad prompts like “improve yourself.”

## Scope Guardrails

Allowed in phase 1:

- bug fixes
- focused tests
- docs updates
- small, local refactors

Blocked by default:

- `.github/workflows/`
- `src/security/`
- `src/runtime/`
- `src/gateway/`
- `scripts/self-improve/`
- deployment/bootstrap/install scripts
- secrets
- systemd/service management
- production configuration
- repository policy files such as `AGENTS.md`, `TOOLS.md`, and `SOUL.md`

Reference denylist:

- `scripts/self-improve/deny_paths.txt`

## Required Validation

Run validation in `candidate` only:

```bash
cd ~/topclaw-self/candidate
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If any step fails, the candidate does not promote.

## Promotion

Promotion is intentionally external. The candidate does not promote itself.

Use:

```bash
scripts/self-improve/promote.sh
```

This script:

1. compares `stable` and `candidate`
2. blocks promotion if any changed path matches `scripts/self-improve/deny_paths.txt`
3. validates the candidate
4. builds a release snapshot
5. copies the candidate into `releases/<timestamp>`
6. backs up the previous stable checkout
7. replaces `stable` with the validated release snapshot
8. writes a promotion log under `logs/`

The deny-path guard is fail-closed. If the denylist file is missing or a blocked path changed, promotion stops before validation or replacement.

## Rollback

If the promoted stable copy is bad, restore the previous snapshot:

```bash
rm -rf ~/topclaw-self/stable
cp -a ~/topclaw-self/stable.backup.<timestamp> ~/topclaw-self/stable
```

Because phase 1 uses full-directory snapshots, rollback is simple and fast.

## Operational Notes

- Do not let TopClaw edit the promotion scripts.
- Do not let TopClaw operate directly in `stable`.
- Do not combine multiple unrelated fixes in one candidate pass.
- Review the candidate diff before promotion, even if the deny-path guard and validation both pass.
- Prefer branch-per-task discipline inside `candidate` if you run multiple iterations.

## Recommended Prompt

Use this template as a starting point:

```text
You now only work on the candidate copy of TopClaw.

Working directory:
~/topclaw-self/candidate

Rules:
1. Only modify files inside the candidate directory.
2. Do not modify service files, systemd units, deployment scripts, update scripts, security policy, runtime policy, default permission config, secrets, or production config.
3. Treat these paths as denied by default:
   - .github/workflows/
   - src/security/
   - src/runtime/
   - src/gateway/
   - scripts/self-improve/
   - AGENTS.md
   - TOOLS.md
   - SOUL.md
4. Prefer one issue at a time.
5. Prefer small bug fixes, tests, docs updates, or small refactors.
6. After changes, run:
   - cargo fmt --all -- --check
   - cargo clippy --all-targets --all-features -- -D warnings
   - cargo test
7. Output:
   - what changed
   - why it changed
   - risk
   - validation results
8. Do not promote, restart, replace, or deploy the stable version.
```

## Non-Goals

Phase 1 explicitly does not include:

- automatic promotion
- automatic service restart
- in-place binary replacement
- automatic systemd edits
- autonomous security-policy changes
- self-modifying promotion logic
