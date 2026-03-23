# TopClaw Docs Structure Map

This page defines the canonical documentation layout.

Last refreshed: **March 23, 2026**.

## 1) Directory Spine (Canonical)

### Layer A: global entry points

- Root product landing: `README.md`
- Docs hub: `docs/README.md`
- Unified TOC: `docs/SUMMARY.md`

### Layer B: category collections (English source-of-truth)

- `docs/getting-started/`
- `docs/reference/`
- `docs/operations/`
- `docs/security/`
- `docs/hardware/`
- `docs/contributing/`
- `docs/project/`
- `docs/sop/`

## 2) Category Intent Map

| Category | Canonical index | Intent |
|---|---|---|
| Getting Started | `docs/getting-started/README.md` | first-run and install flows |
| Reference | `docs/reference/README.md` | commands/config/providers/channels and integration references |
| Operations | `docs/operations/README.md` | day-2 operations, release, troubleshooting runbooks |
| Security | `docs/security/README.md` | current hardening guidance + proposal boundary |
| Hardware | `docs/hardware/README.md` | boards, peripherals, datasheets navigation |
| Contributing | `docs/contributing/README.md` | PR/review/CI policy and process |
| Project | `docs/project/README.md` | current project-level documentation inventory and governance |
| SOP | `docs/sop/README.md` | SOP runtime contract and procedure docs |

## 3) Placement Rules

1. Runtime behavior docs go in English canonical paths first.
2. Every new major doc must be linked from:
   - the nearest category index (`docs/<category>/README.md`)
   - `docs/SUMMARY.md`
   - `docs/docs-inventory.md`
3. Keep compatibility shims aligned when touched; do not introduce new primary content under compatibility-only paths.

## 4) Governance Links

- docs inventory/classification: [../docs-inventory.md](../docs-inventory.md)
