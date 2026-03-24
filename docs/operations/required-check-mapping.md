# Required Check Mapping

This document maps merge-critical workflows to expected check names.

## Merge to `dev` / `main`

| Required check name | Source workflow | Scope |
| --- | --- | --- |
| `CI Required Gate` | `.github/workflows/ci-run.yml` | core Rust/doc merge gate |
| `Security Audit` | `.github/workflows/sec-audit.yml` | dependencies, secrets, governance |
| `Workflow Sanity` | `.github/workflows/workflow-sanity.yml` | workflow syntax and lint |

> `Feature Matrix Summary` was removed in v2026.3.22 along with `feature-matrix.yml`.

## Release / Pre-release

| Required check name | Source workflow | Scope |
| --- | --- | --- |
| `Verify Artifact Set` | `.github/workflows/pub-release.yml` | release completeness |

## Verification Procedure

1. Resolve latest workflow run IDs:
   - `gh run list --repo topway-ai/topclaw --workflow ci-run.yml --limit 1`
2. Enumerate check/job names and compare to this mapping:
   - `gh run view <run_id> --repo topway-ai/topclaw --json jobs --jq '.jobs[].name'`
3. If any merge-critical check name changed, update this file before changing branch protection policy.
4. Export and commit branch/ruleset snapshots as documented in `docs/operations/branch-protection.md`.

## Notes

- Use pinned `uses:` references for all workflow actions.
- Keep check names stable; renaming check jobs can break branch protection rules.
- GitHub scheduled/manual discovery for workflows is default-branch driven. If a release/nightly workflow only exists on a non-default branch, merge it into the default branch before expecting schedule visibility.
- Update this mapping whenever merge-critical workflows/jobs are added or renamed.
