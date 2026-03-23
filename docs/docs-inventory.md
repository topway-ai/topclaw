# TopClaw Documentation Inventory

This inventory classifies documentation by intent and canonical location.

Last reviewed: **March 23, 2026**.

## Classification Legend

- **Current Guide/Reference**: intended to match current runtime behavior
- **Policy/Process**: contribution or governance contract
- **Proposal/Roadmap**: exploratory or planned behavior
- **Snapshot/Audit**: time-bound status and gap analysis
- **Compatibility Shim**: path preserved for backward navigation

## Entry Points

### Product root

| Doc | Type | Audience |
|---|---|---|
| `README.md` | Current Guide | all readers |

### Docs system

| Doc | Type | Audience |
|---|---|---|
| `docs/README.md` | Current Guide (hub) | all readers |
| `docs/SUMMARY.md` | Current Guide (unified TOC) | all readers |
| `docs/structure/README.md` | Current Guide (structure map) | maintainers |

## Collection Index Docs (English canonical)

| Doc | Type | Audience |
|---|---|---|
| `docs/getting-started/README.md` | Current Guide | new users |
| `docs/reference/README.md` | Current Guide | users/operators |
| `docs/operations/README.md` | Current Guide | operators |
| `docs/security/README.md` | Current Guide | operators/contributors |
| `docs/hardware/README.md` | Current Guide | hardware builders |
| `docs/contributing/README.md` | Current Guide | contributors/reviewers |
| `docs/project/README.md` | Current Guide | maintainers |
| `docs/sop/README.md` | Current Guide | operators/automation maintainers |

## Current Guides & References

| Doc | Type | Audience |
|---|---|---|
| `docs/one-click-bootstrap.md` | Current Guide | users/operators |
| `docs/android-setup.md` | Current Guide | Android users/operators |
| `docs/commands-reference.md` | Current Reference | users/operators |
| `docs/providers-reference.md` | Current Reference | users/operators |
| `docs/channels-reference.md` | Current Reference | users/operators |
| `docs/config-reference.md` | Current Reference | operators |
| `docs/custom-providers.md` | Current Integration Guide | integration developers |
| `docs/zai-glm-setup.md` | Current Provider Setup Guide | users/operators |
| `docs/langgraph-integration.md` | Current Integration Guide | integration developers |
| `docs/proxy-agent-playbook.md` | Current Operations Playbook | operators/maintainers |
| `docs/operations-runbook.md` | Current Guide | operators |
| `docs/operations/connectivity-probes-runbook.md` | Current CI/ops Runbook | maintainers/operators |
| `docs/troubleshooting.md` | Current Guide | users/operators |
| `docs/network-deployment.md` | Current Guide | operators |
| `docs/cargo-slicer-speedup.md` | Current Build/CI Guide | maintainers |
| `docs/adding-boards-and-tools.md` | Current Guide | hardware builders |
| `docs/arduino-uno-q-setup.md` | Current Guide | hardware builders |
| `docs/nucleo-setup.md` | Current Guide | hardware builders |
| `docs/hardware-peripherals-design.md` | Current Design Spec | hardware contributors |
| `docs/datasheets/README.md` | Current Hardware Index | hardware builders |
| `docs/datasheets/nucleo-f401re.md` | Current Hardware Reference | hardware builders |
| `docs/datasheets/arduino-uno.md` | Current Hardware Reference | hardware builders |
| `docs/datasheets/esp32.md` | Current Hardware Reference | hardware builders |
| `docs/audit-event-schema.md` | Current CI/Security Reference | maintainers/security reviewers |

## Policy / Process Docs

| Doc | Type |
|---|---|
| `docs/pr-workflow.md` | Policy |
| `docs/reviewer-playbook.md` | Process |
| `docs/ci-map.md` | Process |
| `docs/actions-source-policy.md` | Policy |

## Proposal / Roadmap Docs

These are valuable context, but **not strict runtime contracts**.

| Doc | Type |
|---|---|
| `docs/sandboxing.md` | Proposal |
| `docs/resource-limits.md` | Proposal |
| `docs/audit-logging.md` | Proposal |
| `docs/agnostic-security.md` | Proposal |
| `docs/frictionless-security.md` | Proposal |
| `docs/security-roadmap.md` | Roadmap |

## Maintenance Contract

1. Update `docs/SUMMARY.md` and nearest category index when adding a major doc.
2. Keep proposal/roadmap docs explicitly labeled; avoid mixing proposal text into runtime-contract docs.
