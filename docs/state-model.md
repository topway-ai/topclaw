# `.topclaw` State Model — Current-Version-Only

This document maps every persistent file and directory under `~/.topclaw`,
classifies its lifecycle, and identifies state that can be narrowed or removed
under the current-version-only state model (no backward-compatibility fallbacks
for older layouts).

## Directory Structure

```
~/.topclaw/
├── config.toml                          # Primary configuration file
├── .secret_key                          # Encryption key for at-rest secrets
├── active_workspace.toml                # Workspace marker (which profile is active)
├── estop-state.json                     # Emergency stop state
├── browser-allowed-domains-grants.json  # Persistent browser domain approvals
├── workspace/                           # Default workspace directory
│   └── skills/                          # Skill definitions
│       └── <name>/
│           └── SKILL.md
├── repositories/
│   └── topclaw/                         # Curated skills repo checkout
├── state/                               # Runtime state directory
│   └── runtime-trace.jsonl              # Runtime trace log
└── [per-workspace config dirs]          # Named workspace profiles
    └── config.toml
```

## State Classification

Each entry is classified by:

- **Scope**: `global` (shared across workspaces) or `workspace` (per-profile)
- **Mutability**: `static` (written once, read many), `dynamic` (frequently updated), or `ephemeral` (can be regenerated)
- **Criticality**: `essential` (data loss breaks functionality), `important` (degraded experience), or `cosmetic` (nice-to-have)

| Path | Scope | Mutability | Criticality | Owner module | Description |
|---|---|---|---|---|---|
| `config.toml` | workspace | dynamic | essential | `config` | Main config file, encrypted at rest |
| `.secret_key` | global | static | essential | `security::secrets` | AES-256 key for config encryption |
| `active_workspace.toml` | global | dynamic | important | `config::schema_runtime_dirs` | Points to active workspace config dir |
| `estop-state.json` | global | dynamic | important | `config::estop` | Emergency stop flag + timestamp |
| `browser-allowed-domains-grants.json` | global | dynamic | important | `config::browser_domain_grants` | User-approved browser domains |
| `workspace/` | global | dynamic | essential | `config` | Default workspace root |
| `workspace/skills/` | global | dynamic | important | `skills` | User skill definitions |
| `repositories/topclaw/` | global | ephemeral | cosmetic | `skills` | Curated skills repo (can re-clone) |
| `state/runtime-trace.jsonl` | global | ephemeral | cosmetic | `observability` | Debug trace log (auto-pruned) |

## Removed State (Legacy Cleanup)

The following state paths were part of a legacy layout that has been removed
in the current-version-only state model:

| Path | Status | Reason |
|---|---|---|
| `../.topclaw/config.toml` (parent lookup) | **Removed** | `resolve_config_dir_for_workspace()` no longer falls back to `../.topclaw/config.toml`. Users must place config in the workspace directory directly. |
| `TOPCLAW_WORKSPACE=<path>/workspace` → `../.topclaw/` | **Removed** | The legacy suffix-detection path that checked the parent directory for `.topclaw/` has been excised. |

## State Resolution Order

The current state resolution order (no legacy fallbacks):

1. **`TOPCLAW_WORKSPACE` env var** — If set, the value is the workspace
   directory. Config is looked for at `<workspace>/config.toml`. If not found,
   the workspace directory itself becomes the config root (and `config.toml`
   is written there on first `load_or_init`).

2. **`TOPCLAW_CONFIG_DIR` env var** — If set, this is the config directory
   directly. Overrides `TOPCLAW_WORKSPACE` for config-only operations.

3. **`active_workspace.toml` marker** — If present in `~/.topclaw/`, points
   to the active workspace config directory.

4. **Default** — `~/.topclaw/` is the config directory.

### Two-Case `resolve_config_dir_for_workspace`

When a workspace directory is given (via env or marker):

1. **Workspace contains `config.toml`** → use it: `(workspace_dir, workspace_dir/workspace)`
2. **No `config.toml` found** → treat workspace as config root: `(workspace_dir, workspace_dir/workspace)`

Both cases produce the same result, which is the simplification from removing
the legacy `../.topclaw/` fallback.

## Narrowing Opportunities

The following state can be further narrowed in future versions:

1. **`estop-state.json`** — Currently a standalone file. Could be merged into
   `config.toml` as a `[estop]` section or into `active_workspace.toml` as an
   additional field, reducing the number of top-level files.

2. **`browser-allowed-domains-grants.json`** — Currently a standalone file.
   Could be merged into `config.toml` as a `[browser.granted_domains]` section
   or into `active_workspace.toml`. This would eliminate a separate
   read-modify-write path.

3. **`repositories/topclaw/`** — Ephemeral; can be re-cloned. Could be moved
   to `~/.cache/topclaw/` to separate cache from state, following XDG
   conventions.

4. **`state/runtime-trace.jsonl`** — Ephemeral; auto-pruned by max entries.
   Could be moved to `~/.cache/topclaw/` for XDG compliance.

5. **`.secret_key`** — The encryption key file is separate from config for
   security (different file permissions). This should remain a standalone
   file but could benefit from a `.secret_key.age` variant for age-encrypted
   keys in multi-user scenarios.

## Security Properties

| Property | Enforcement |
|---|---|
| Config file permissions | `0600` on Unix (enforced at save time) |
| Secret key permissions | `0600` on Unix |
| API key at-rest encryption | AES-256-GCM via `SecretStore` when `secrets.encrypt = true` |
| Debug impl credential redaction | Manual `Debug` impl on `BrowserComputerUseConfig` and `Config` omit `api_key` fields |
| Domain grants integrity | JSON file, overwritten atomically |
