# TopClaw Operations Runbook

This runbook is for operators who maintain availability, security posture, and incident response.

Last verified: **February 18, 2026**.

## Scope

Use this document for day-2 operations:

- starting and supervising runtime
- health checks and diagnostics
- safe rollout and rollback
- incident triage and recovery

For first-time installation, start from [one-click-bootstrap.md](one-click-bootstrap.md).

## Runtime Modes

| Mode | Command | When to use |
|---|---|---|
| Foreground runtime | `topclaw daemon` | local debugging, short-lived sessions |
| Foreground gateway only | `topclaw gateway` | webhook endpoint testing |
| User service | `topclaw service install && topclaw service start` | persistent operator-managed runtime |

## Baseline Operator Checklist

1. Validate configuration:

```bash
topclaw status
```

2. Verify diagnostics:

```bash
topclaw doctor
topclaw channel doctor
```

3. Start runtime:

```bash
topclaw daemon
```

4. For persistent user session service:

```bash
topclaw service install
topclaw service start
topclaw service status
```

## Health and State Signals

| Signal | Command / File | Expected |
|---|---|---|
| Config validity | `topclaw doctor` | no critical errors |
| Channel connectivity | `topclaw channel doctor` | configured channels healthy |
| Runtime summary | `topclaw status` | expected provider/model/channels |
| Daemon heartbeat/state | `~/.topclaw/daemon_state.json` | file updates periodically |

## Logs and Diagnostics

### macOS / Windows (service wrapper logs)

- `~/.topclaw/logs/daemon.stdout.log`
- `~/.topclaw/logs/daemon.stderr.log`

### Linux (systemd user service)

```bash
journalctl --user -u topclaw.service -f
```

## Incident Triage Flow (Fast Path)

1. Snapshot system state:

```bash
topclaw status
topclaw doctor
topclaw channel doctor
```

2. Check service state:

```bash
topclaw service status
```

3. If service is unhealthy, restart cleanly:

```bash
topclaw service stop
topclaw service start
```

4. If channels still fail, verify allowlists and credentials in `~/.topclaw/config.toml`.

5. If gateway is involved, verify bind/auth settings (`[gateway]`) and local reachability.

## Secret Leak Incident Response (CI Gitleaks)

When `sec-audit.yml` reports a gitleaks finding or uploads SARIF alerts:

1. Confirm whether the finding is a true credential leak or a test/doc false positive:
   - review `gitleaks.sarif` + `gitleaks-summary.json` artifacts
   - inspect changed commit range in the workflow summary
2. If true positive:
   - revoke/rotate the exposed secret immediately
   - remove leaked material from reachable history when required by policy
   - open an incident record and track remediation ownership
3. If false positive:
   - prefer narrowing detection scope first
   - only add allowlist entries with explicit governance metadata (`owner`, `reason`, `ticket`, `expires_on`)
   - ensure the related governance ticket is linked in the PR
4. Re-run `Sec Audit` and confirm:
   - gitleaks lane green
   - governance guard green
   - SARIF upload succeeds

## Safe Change Procedure

Before applying config changes:

1. backup `~/.topclaw/config.toml`
2. apply one logical change at a time
3. run `topclaw doctor`
4. restart daemon/service
5. verify with `status` + `channel doctor`

## Rollback Procedure

If a rollout regresses behavior:

1. restore previous `config.toml`
2. restart runtime (`daemon` or `service`)
3. confirm recovery via `doctor` and channel health checks
4. document incident root cause and mitigation

## Related Docs

- [one-click-bootstrap.md](one-click-bootstrap.md)
- [troubleshooting.md](troubleshooting.md)
- [config-reference.md](config-reference.md)
- [commands-reference.md](commands-reference.md)
