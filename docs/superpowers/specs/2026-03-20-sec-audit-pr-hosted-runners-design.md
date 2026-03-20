# Sec Audit PR Hosted Runners Design

Date: 2026-03-20
Status: Approved for planning
Scope: Move only the merge-blocking `Sec Audit` PR lane to GitHub-hosted runners while preserving the existing branch-protection contract.

## Problem

`Sec Audit` is currently fully pinned to self-hosted runners. The repository currently has zero available self-hosted runners, so pull requests can block indefinitely waiting for the required `Security Required Gate` check to start. That prevents normal merges even when the code is otherwise ready.

The actual problem is narrower than "migrate all security CI." The merge-blocking path is the urgent surface. Push, schedule, and manual security analysis are still useful, but they do not need to be redesigned to restore contributor flow.

## Goals

- Preserve the visible required status name `Security Required Gate`.
- Restore PR and merge-queue mergeability without relying on self-hosted runner availability.
- Keep the existing `Sec Audit` workflow file as the single entry point.
- Keep non-PR `Sec Audit` coverage behavior unchanged.
- Keep the change easy to revert in one workflow rollback.

## Non-Goals

- Do not migrate the entire `Sec Audit` workflow to GitHub-hosted runners.
- Do not change branch-protection rules or required status names.
- Do not redesign the security job contents unless GitHub-hosted compatibility forces a narrow follow-up.
- Do not introduce a reusable workflow refactor in this change.

## Options Considered

### Option A: Single workflow, split by event class inside the same file

Keep `.github/workflows/sec-audit.yml` as the only workflow. Run the merge-blocking lane on `ubuntu-latest` for `pull_request` and `merge_group`. Keep `push`, `schedule`, and `workflow_dispatch` on the current self-hosted labels.

Pros:

- Smallest operational change.
- No branch-protection edits.
- Clear rollback path.

Cons:

- Some YAML duplication between hosted and self-hosted lanes.

### Option B: Separate PR-only hosted workflow

Create a second workflow dedicated to `pull_request` and `merge_group`, leaving the existing self-hosted workflow for other events.

Pros:

- Cleaner event separation.

Cons:

- More workflow sprawl.
- Higher sync burden across two workflow files.

### Option C: Reusable workflow plus two thin wrappers

Extract shared steps into a reusable workflow and call it from hosted and self-hosted wrappers.

Pros:

- Best long-term DRY story.

Cons:

- Highest churn and complexity for an operational unblock.

## Chosen Design

Use Option A.

Keep `Sec Audit` in a single workflow file and split execution by event class. The PR-facing lane moves to GitHub-hosted runners. The non-PR lane stays on self-hosted runners. The required gate remains `Security Required Gate` so branch protection does not need to change.

## Workflow Topology

The workflow will keep the same triggers:

- `pull_request`
- `merge_group`
- `push`
- `schedule`
- `workflow_dispatch`

The jobs will be split into two families:

1. Hosted PR lane

- Runs only for `pull_request` and `merge_group`.
- Uses `ubuntu-latest`.
- Includes the full dependency chain that feeds the required gate:
  - RustSec audit
  - cargo-deny
  - security regression checks
  - secret scanning
  - SBOM generation
  - unsafe debt check
  - `Security Required Gate`

2. Self-hosted non-PR lane

- Runs only for `push`, `schedule`, and `workflow_dispatch`.
- Keeps the current self-hosted labels and step bodies.
- Preserves the existing non-blocking security coverage behavior.

## Naming and Branch Protection Contract

- The PR-visible gate job keeps the exact visible name `Security Required Gate`.
- The workflow name stays `Sec Audit`.
- The hosted PR lane is the only lane that must emit the required gate on pull requests and merge groups.
- Non-PR jobs may use distinct job ids or descriptive names as needed, but the required PR-visible check name must stay unchanged.

## Why the Whole PR Lane Must Move

Moving only the final gate job would not solve the problem. `Security Required Gate` depends on upstream security jobs. If those upstream jobs remain on self-hosted runners, the gate still blocks waiting for unavailable infrastructure.

So the unit of migration is the merge-blocking lane, not just the final gate job.

## Implementation Notes

- Prefer explicit hosted and self-hosted job families with `if:` guards over clever `runs-on` expressions. That keeps runner placement auditable and avoids brittle array-or-string expression tricks in GitHub Actions YAML.
- Keep job steps unchanged first. Only add explicit setup if GitHub-hosted runners expose a concrete missing dependency.
- Preserve artifacts and SARIF upload behavior unless the hosted environment requires a narrow adjustment.
- Keep the required gate dependency graph explicit so reviewers can confirm that every PR-blocking prerequisite now runs on GitHub-hosted infrastructure.

## Documentation Impact

Update these docs during implementation:

- `docs/ci-map.md`
- `docs/pr-workflow.md` if it needs a short operational note

No locale-navigation or docs-IA change is expected from this work. No i18n follow-through should be required unless implementation expands beyond internal CI/contributor wording.

## Validation Plan

Local validation:

- Validate `.github/workflows/sec-audit.yml` syntax after the workflow edit.
- Run the most relevant repository checks if helper scripts change.

Post-merge or PR validation:

- Confirm a PR-triggered `Sec Audit` run starts on GitHub-hosted runners.
- Confirm the visible required status remains `Security Required Gate`.
- Confirm `push`, `schedule`, or manual dispatch still route to self-hosted runners.

## Risks

- GitHub-hosted runners may not have an implicitly available dependency that self-hosted runners currently provide.
- Hosted runners may increase runtime for some security jobs.
- Duplicate YAML paths can drift if future edits are applied to only one lane.

## Mitigations

- Keep the first implementation narrow and preserve existing steps where possible.
- If a hosted job fails due to a missing tool, fix that tool setup explicitly inside the workflow.
- Keep the change isolated to one workflow file plus minimal docs updates.

## Rollback

Rollback is a single revert of the `sec-audit.yml` change and any associated docs update. Because branch-protection names remain unchanged, rollback does not require repository settings changes.

## Open Questions Resolved

- Preserve branch-protection contract: yes.
- Preserve required status name `Security Required Gate`: yes.
- Scope of runner migration: only the merge-blocking PR and merge-group lane.
