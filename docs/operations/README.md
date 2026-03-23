# Operations

Use this section once TopClaw is installed and you need to run, debug, or maintain it over time.

## Core Pages

| Need | Read this |
|---|---|
| Day-2 runtime operations | [../operations-runbook.md](../operations-runbook.md) |
| Choose the right runtime mode first | [../runtime-model.md](../runtime-model.md) |
| Heartbeat scheduling | [heartbeat-runbook.md](heartbeat-runbook.md) |
| Troubleshooting | [../troubleshooting.md](../troubleshooting.md) |
| Computer-use sidecars | [computer-use-sidecar-runbook.md](computer-use-sidecar-runbook.md) |
| Connectivity probes | [connectivity-probes-runbook.md](connectivity-probes-runbook.md) |
| Release workflow | [../release-process.md](../release-process.md) |
| Network deployment | [../network-deployment.md](../network-deployment.md) |

## Recommended Workflow

1. Check current state with `topclaw status` and `topclaw status --diagnose`
2. Change one config area at a time
3. Restart the relevant process
4. Verify provider, channel, and gateway health
5. Roll back quickly if behavior regresses

## Related

- Config reference: [../config-reference.md](../config-reference.md)
- Security docs: [../security/README.md](../security/README.md)
