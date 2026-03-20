# Sec Audit PR Hosted Runners Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move only the merge-blocking `Sec Audit` PR and merge-group lane onto GitHub-hosted runners while preserving the existing `Security Required Gate` branch-protection contract.

**Architecture:** Keep `.github/workflows/sec-audit.yml` as the single workflow entry point, but split its jobs into two event-specific families: a GitHub-hosted lane for `pull_request` and `merge_group`, and a self-hosted lane for `push`, `schedule`, and `workflow_dispatch`. Lock that contract with focused regression tests in the existing CI script test suite, then update the small set of docs that describe merge-gating behavior.

**Tech Stack:** GitHub Actions YAML, Python `unittest`, repository CI docs, Ruby YAML parser for local syntax validation

---

## File Map

- `.github/workflows/sec-audit.yml`
  - Single source of truth for the security audit workflow.
  - Will gain explicit hosted PR/merge-group jobs and explicit self-hosted non-PR jobs.
- `scripts/ci/tests/test_ci_scripts.py`
  - Existing home for CI workflow and helper-script regression tests.
  - Will gain focused assertions that the `Sec Audit` workflow preserves `Security Required Gate` while routing the merge-blocking lane to GitHub-hosted runners.
- `docs/ci-map.md`
  - Operator-facing CI workflow map.
  - Must explain that `Sec Audit` now splits runner placement by event class.
- `docs/pr-workflow.md`
  - Contributor-facing merge/readiness workflow.
  - Update only if the current wording needs a short operational note to avoid implying self-hosted runner dependence for required security checks.

## Task 1: Lock the Runner-Split Contract with Failing Tests

**Files:**
- Modify: `scripts/ci/tests/test_ci_scripts.py`
- Test: `.github/workflows/sec-audit.yml`

- [ ] **Step 1: Add focused failing tests for the workflow contract**

Add two tests to `CiScriptsBehaviorTest` that read `.github/workflows/sec-audit.yml` and assert:
- the PR/merge-group lane uses `ubuntu-latest`
- the non-PR lane still contains the current self-hosted labels
- the visible required gate name `Security Required Gate` is still present

Use simple text assertions or lightweight pattern matching against the real workflow file. Do not introduce a new YAML-parsing dependency just for this check.

- [ ] **Step 2: Run the targeted tests to verify they fail against the current workflow**

Run:

```bash
python3 -m unittest \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_pr_lane_uses_github_hosted_runners \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_non_pr_lane_keeps_self_hosted_runners
```

Expected:
- at least one test fails because `sec-audit.yml` still runs the merge-blocking lane on self-hosted runners only

- [ ] **Step 3: Keep the tests narrowly scoped**

Before moving on, confirm the tests assert runner placement and gate-name preservation only. Do not broaden them into full workflow snapshots that will create noisy future churn.

- [ ] **Step 4: Commit the test contract**

```bash
git add scripts/ci/tests/test_ci_scripts.py
git commit -m "test(ci): lock sec audit runner split contract"
```

## Task 2: Move the Merge-Blocking Security Lane to GitHub-Hosted Runners

**Files:**
- Modify: `.github/workflows/sec-audit.yml`
- Test: `scripts/ci/tests/test_ci_scripts.py`

- [ ] **Step 1: Split the workflow into hosted and self-hosted job families**

In `.github/workflows/sec-audit.yml`, duplicate the current merge-blocking lane into hosted PR/merge-group jobs and keep separate self-hosted jobs for non-PR events.

Implementation rules:
- preserve the workflow name `Sec Audit`
- preserve the visible required gate name `Security Required Gate`
- keep `pull_request` and `merge_group` prerequisites on `ubuntu-latest`
- keep `push`, `schedule`, and `workflow_dispatch` prerequisites on the existing self-hosted labels
- keep the dependency graph explicit so the hosted `security-required` gate depends only on hosted upstream jobs

- [ ] **Step 2: Preserve step bodies first**

Copy the existing job steps as-is into the hosted lane before making any environment adjustments. Only add explicit setup if GitHub-hosted runners expose a concrete missing dependency during verification.

- [ ] **Step 3: Run the targeted workflow-contract tests**

Run:

```bash
python3 -m unittest \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_pr_lane_uses_github_hosted_runners \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_non_pr_lane_keeps_self_hosted_runners
```

Expected:
- PASS

- [ ] **Step 4: Validate workflow YAML syntax**

Run:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/sec-audit.yml"); puts "sec-audit.yml OK"'
```

Expected:
- prints `sec-audit.yml OK`

- [ ] **Step 5: Commit the workflow migration**

```bash
git add .github/workflows/sec-audit.yml scripts/ci/tests/test_ci_scripts.py
git commit -m "ci: move sec audit PR lane to github-hosted runners"
```

## Task 3: Update Workflow Documentation and Final Verification

**Files:**
- Modify: `docs/ci-map.md`
- Modify: `docs/pr-workflow.md`
- Test: `.github/workflows/sec-audit.yml`

- [ ] **Step 1: Update the CI workflow map**

Edit `docs/ci-map.md` so the `Sec Audit` entry states:
- PR and merge-queue runs use GitHub-hosted runners
- push, schedule, and manual runs stay on self-hosted runners
- `Security Required Gate` remains the merge gate

- [ ] **Step 2: Make the PR workflow wording consistent**

Review `docs/pr-workflow.md` and add only the smallest wording change needed so it does not imply that required security checks still depend on self-hosted runner availability. If the current text is already accurate without that detail, keep the edit minimal or omit unnecessary prose churn.

- [ ] **Step 3: Run focused verification again after docs are aligned**

Run:

```bash
python3 -m unittest \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_pr_lane_uses_github_hosted_runners \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_non_pr_lane_keeps_self_hosted_runners
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/sec-audit.yml"); puts "sec-audit.yml OK"'
```

Expected:
- targeted tests PASS
- YAML validation prints `sec-audit.yml OK`

- [ ] **Step 4: Review the scoped diff before committing**

Run:

```bash
git status --short
git diff -- .github/workflows/sec-audit.yml docs/ci-map.md docs/pr-workflow.md scripts/ci/tests/test_ci_scripts.py
```

Expected:
- only the workflow, targeted tests, and small docs updates are present

- [ ] **Step 5: Commit the docs alignment**

```bash
git add docs/ci-map.md docs/pr-workflow.md .github/workflows/sec-audit.yml scripts/ci/tests/test_ci_scripts.py
git commit -m "docs: align security workflow runner guidance"
```

## Final Verification Checklist

- [ ] Run the focused workflow-contract tests:

```bash
python3 -m unittest \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_pr_lane_uses_github_hosted_runners \
  scripts.ci.tests.test_ci_scripts.CiScriptsBehaviorTest.test_sec_audit_non_pr_lane_keeps_self_hosted_runners
```

- [ ] Validate workflow syntax:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/sec-audit.yml"); puts "sec-audit.yml OK"'
```

- [ ] Run the repo’s workflow sanity path if available in the local environment; otherwise note that GitHub Actions will provide the authoritative `Workflow Sanity` check.

- [ ] After opening the PR, confirm one PR-triggered `Sec Audit` run starts on GitHub-hosted runners and still emits `Security Required Gate`.

## Notes for the Implementer

- Keep the patch reversible. Do not refactor other workflows while touching `sec-audit.yml`.
- Prefer duplication over clever YAML indirection for this change.
- If GitHub-hosted runners are missing a tool that self-hosted had implicitly, add explicit setup in the hosted lane rather than widening the migration.
- Do not rename the required gate job or change branch-protection assumptions.
