# TopClaw Simplification Plan

> **Historical snapshot.** This plan was written on 2026-03-18 and largely executed in v2026.3.22. Phantom channel flags, WhatsApp Web deps, and dead SOP tools have been removed. See `audit-2026-03-22-simplification.md` for the execution record.

**Date**: 2026-03-18
**Phase**: Phase 2 - Implementation Strategy

---

## Executive Summary

Based on the Phase 1 audit, the codebase has:
- **193,924 total lines** across 50 modules
- **7,682 lines** in the config schema alone (40+ subsections)
- **11,545 lines** in the channel routing module
- Good feature flag discipline already in place
- Several niche subsystems that add compile-time and cognitive overhead

## Simplification Objectives

1. **Reduce cognitive load** for new maintainers
2. **Decrease compile times** for default builds
3. **Narrow public surfaces** where possible
4. **Preserve core functionality** and extension points
5. **Remove dead weight** without breaking working flows

---

## Plan A: Immediate Wins (Low Risk)

### A.1: Clean Up Examples

**Current state**: 4 relevant examples + 4 platform-specific computer_use stubs that are ~23KB each

| Action | Files | Rationale | Risk |
|--------|-------|----------|------|
| Remove platform-specific stubs | `examples/computer_use_sidecar_*.rs` | These are 23KB+ stubs, not real examples | Low |

**Rationale**: These are platform-specific stubs that aren't usable as-is and clutter the examples directory.

---

### A.2: Simplify Workspace Crate Dependencies

**Current state**: `crates/robot-kit` has commented-out `topclaw` dependency

| Action | Files | Rationale | Risk |
|--------|-------|----------|------|
| Remove commented dependency | `crates/robot-kit/Cargo.toml` | Dead code | Very Low |

---

### A.3: Remove Redundant Alias Resolution Functions

**Current state**: `src/providers/mod.rs` has ~90 lines of provider alias functions

| Action | Files | Rationale | Risk |
|--------|-------|----------|------|
| Evaluate if alias resolution can be simplified | `src/providers/alias.rs` | Aliases exist for user convenience | Medium |

**Note**: Provider aliases serve real provider diversity needs. Be careful not to break existing configurations.

---

## Plan B: Config Schema Refactoring (Medium Risk)

### B.1: Split Config Schema into Modules

**Current state**: Single `src/config/schema.rs` with 7,682 lines and 40+ subsections

**Proposed structure**:
```
src/config/
├── mod.rs           # Re-exports and Config struct
├── agent.rs         # AgentConfig
├── provider.rs      # ProviderConfig, ModelRouteConfig
├── channel.rs       # ChannelConfig
├── gateway.rs       # GatewayConfig
├── memory.rs        # MemoryConfig
├── security.rs      # SecurityConfig, EstopConfig
├── cron.rs          # CronConfig
├── observability.rs # ObservabilityConfig
├── hardware.rs      # HardwareConfig, PeripheralsConfig
├── schema.rs        # Keep generic types, enums
└── ...
```

| Change | Rationale | Risk |
|--------|----------|------|
| Split schema.rs into focused modules | Easier to navigate, clearer ownership | Medium - may break external users |
| Keep Config struct in mod.rs | Public API surface |

**Implementation approach**:
1. Create new module files
2. Move config structs with `pub use` in mod.rs
3. Maintain backward compatibility for external users
4. Add deprecation warnings for moved items

---

### B.2: Reduce Config Subsections

**Current state**: 40+ config subsections, many with single-digit field counts

**High-value consolidations**:
| Current | Proposed | Rationale |
|---------|----------|-----------|
| `self_improvement` (50+ lines) | Keep separate | Self-contained feature |
| `goal_loop` (20+ lines) | Merge into `agent` | Tied to agent behavior |
| `research` (15+ lines) | Merge into `agent` | Agent research mode |
| `otp` (15+ lines) | Merge into `security` | Security-related |

---

## Plan C: Channel Modularization (Medium Risk)

### C.1: Reduce Channel Module Size

**Current state**: `src/channels/mod.rs` at 11,545 lines

**Proposed changes**:
| Action | Rationale | Risk |
|--------|----------|------|
| Move channel factory to `factory.rs` | Already exists, expand usage | Low |
| Move runtime helpers to `runtime_helpers.rs` | Already exists | Low |
| Extract route state to `route_state.rs` | Already exists | Low |
| Create `channel_traits.rs` | Consolidate channel behavior | Medium |

**Channel-specific modules to keep isolated**:
- `telegram.rs` (5,806 lines) - Largest single channel
- `discord.rs` (2,044 lines) - Feature-rich
- `lark.rs` (2,715 lines) - Prost dependency

---

### C.2: Channel Feature Flags Review

**Current channels** (18+):
- Core: `channel-telegram`
- Standard: `channel-discord`, `channel-slack`, `channel-lark`
- Optional: All others

**Recommendation**: The feature flag system is working well. No changes needed.

---

## Plan D: SOP/Coordination Isolation (Medium Risk)

### D.1: SOP Subsystem

**Current state**: 5,641 lines across 7 files

| File | Lines | Purpose |
|------|-------|---------|
| `engine.rs` | 1,631 | Core orchestration |
| `types.rs` | 15K | Type definitions |
| `dispatch.rs` | 26K | Event dispatching |
| `metrics.rs` | 51K | Metrics collection |
| `condition.rs` | 13K | Condition evaluation |
| `gates.rs` | 26K | Gate evaluation |
| `audit.rs` | 10K | Audit logging |

**Recommendations**:
| Action | Rationale | Risk |
|--------|----------|------|
| Make `sop` a feature-gated crate | Already behind `ampersona-gates` | Medium |
| Extract `metrics.rs` to observability | Metrics are generic | Medium |

---

### D.2: Coordination Subsystem

**Current state**: 2,448 lines

**Recommendations**:
| Action | Rationale | Risk |
|--------|----------|------|
| Keep as-is | Feature-gated via tools | Low |
| Consider extraction if usage is low | Niche multi-agent feature | Medium |

---

## Plan E: Remove Niche Subsystems (Low-Medium Risk)

### E.1: SkillForge Evaluation

**Current state**: `src/skillforge/` - Auto-discovery engine

| Action | Rationale | Risk |
|--------|----------|------|
| Mark as experimental | Not core functionality | Low |
| Consider extraction to separate crate | Complex for what it does | Medium |

---

### E.2: Self-Improvement Pipeline

**Current state**: `src/self_improvement/` - Self-improvement job system

| Action | Rationale | Risk |
|--------|----------|------|
| Keep as-is | Useful for CI automation | Low |
| Feature-gate if unused | Currently always compiled | Low |

---

## Plan F: Dependency Review (Medium Risk)

### F.1: Heavy Dependencies

| Dependency | Usage | Recommendation |
|------------|-------|----------------|
| `matrix-sdk` | Matrix channel only | Already feature-gated |
| `wa-rs*` (7 packages) | WhatsApp Web only | Already feature-gated |
| `nostr-sdk` | Nostr channel | Already feature-gated |
| `wasmi` | WASM runtime | Already feature-gated |
| `probe-rs` | Hardware debug | Already feature-gated |
| `opentelemetry*` | Observability | Already feature-gated |

**Status**: These are already well-gated. No changes needed.

---

### F.2: Default Dependencies to Review

| Dependency | Current Usage | Recommendation |
|------------|---------------|----------------|
| `reqwest` | HTTP client | Keep - essential |
| `tokio` | Async runtime | Keep - essential |
| `serde` | Serialization | Keep - essential |
| `rusqlite` | SQLite memory | Keep - essential |
| `chrono` | Time handling | Keep - essential |

---

## Implementation Priority

| Priority | Changes | Estimated Impact |
|----------|---------|------------------|
| **P1 - Now** | Clean up examples | Low cognitive load |
| **P2 - Sprint** | Config schema modularization | High impact on navigability |
| **P3 - Sprint** | Channel module organization | Medium impact |
| **P4 - Later** | SOP/coordination extraction | Long-term maintainability |

---

## Risk Assessment Matrix

| Change | Risk | Benefit | Effort |
|--------|------|---------|--------|
| Clean up examples | Very Low | Low | Low |
| Config schema split | Medium | High | Medium |
| Channel modularization | Medium | Medium | Medium |
| SOP extraction | Medium | Medium | High |
| Dependency cleanup | Low | Low | Low |

---

## Compatibility Considerations

### Items that MUST NOT Change:
1. **CLI command structure** - Users depend on it
2. **Config TOML keys** - Breaking existing configurations
3. **Public trait definitions** - Extension point API
4. **Feature flag names** - Users may depend on them
5. **Runtime behavior** - Core agent loop must work the same

### Items that CAN Change:
1. **Internal module organization** - No external consumers
2. **Config struct organization** - As long as TOML keys work
3. **Private implementation details** - No API guarantees
4. **Compile-time dependencies** - User controls via features

---

## Success Metrics

After simplification, the codebase should have:
- [ ] Clearer module boundaries
- [ ] Smaller average module size
- [ ] Easier onboarding for new maintainers
- [ ] Maintained or improved compile times
- [ ] Preserved functionality
- [ ] No breaking changes to user-facing APIs

---

## Deferred Items

The following items are identified but deferred due to complexity:

1. **Extracting SOP to separate crate** - Requires significant refactoring
2. **Extracting skillforge to separate crate** - Unclear if worth the effort
3. **Full dependency audit** - Requires testing each feature combination
4. **WhatsApp Web cleanup** - Complex optional integration

---

## Implementation Log

### 2026-03-18 - Phase 3 Execution

#### Phase 3: Dead Code Removal ✅

- [x] **Removed skillforge module** - 4 files (~1,118 lines)
  - `src/skillforge/mod.rs`
  - `src/skillforge/scout.rs`
  - `src/skillforge/evaluate.rs`
  - `src/skillforge/integrate.rs`
  
- [x] **Removed SOP tool dead code** - 5 files (~1,672 lines)
  - `src/tools/sop_advance.rs`
  - `src/tools/sop_approve.rs`
  - `src/tools/sop_execute.rs`
  - `src/tools/sop_list.rs`
  - `src/tools/sop_status.rs`

**Total dead code removed: ~2,790 lines across 9 files**

**Validation:**
- `cargo check` ✓
- `cargo check --all-features` ✓
- `cargo test --lib` - 3818 tests passed ✓

#### Plan A: Immediate Wins ✅
- [x] **A.1**: Remove platform-specific computer_use stubs (4 files, ~92KB removed)
- [x] **A.2**: Remove commented dependency in robot-kit
- [x] **A.3**: Evaluated alias resolution - **Keep as-is** (serves user convenience for provider names)

#### Plan B: Config Schema Refactoring
- [x] **B.1**: Added section headers to schema.rs for better navigability
  - SECTION 1: Storage & Memory Configs
  - SECTION 2: Channel Configs
  - SECTION 3: Security Config
  - SECTION 4: Tests
- [ ] **B.2**: Split config schema into focused modules (deferred - risky without more analysis)

#### Plan C: Channel Modularization ✅ (ANALYZED)
- [x] **C.1**: Analyzed channel module size - Already well-structured with 5 sections
  - SECTION 1: Imports and Type Aliases
  - SECTION 2: Constants and Helper Functions  
  - SECTION 3: Message Processing
  - SECTION 4: System Prompt Building
  - SECTION 5: CLI Commands and Channel Startup
  - SECTION 6: Tests (added)
- [x] **C.2**: Channel feature flags review completed
  - Finding: Only 3 channels are properly feature-gated (matrix, lark, whatsapp-web)
  - Most "channel-*" flags are documentation markers only
  - Recommendation: Keep current design - adds complexity to gate all channels
  - Binary size impact is manageable with current approach

#### Plan D: SOP/Coordination Isolation ⚠️ (ANALYZED)
- [x] **D.1**: SOP subsystem analyzed
  - 7 files, ~6.6K LOC total
  - `gates.rs` is behind `ampersona-gates` feature flag (only gated module)
  - **CRITICAL FINDING: 5 SOP tool files are DEAD CODE** (sop_advance.rs, sop_approve.rs, sop_execute.rs, sop_list.rs, sop_status.rs)
    - These tools are defined but NEVER registered in the tool registry
    - Only used in tests
    - ~1,500 lines of dead code
- [x] **D.2**: Coordination subsystem analyzed
  - ~2.4K LOC, used by delegate tools
  - Actively used, keep as-is

#### Plan E: Niche Subsystems ⚠️ (ANALYZED)
- [x] **E.1**: SkillForge evaluation
  - **CRITICAL FINDING: skillforge module is DEAD CODE**
  - Module exists but is not declared as a module anywhere
  - Never exported or used
  - 4 files, ~1.1K LOC of unused code
- [x] **E.2**: Self-improvement pipeline evaluation
  - **ACTIVE** - SelfImprovementTaskTool is registered and used
  - Keep as-is

#### Plan F: Dependency Review ✅

- [x] **F.1**: Heavy dependencies review - **Completed**
  - All optional dependencies are properly feature-gated
  - No unnecessary dependencies found

- [x] **F.2**: Default dependencies review - **Completed**
  - Found and removed duplicate `tempfile` entry in Cargo.toml (line 175)
  - This duplicate was always compiled regardless of features

**Dependency Summary:**
| Category | Count | Status |
|----------|-------|--------|
| Core dependencies | ~40 | Essential - keep |
| Optional dependencies | ~20 | Properly feature-gated |
| Workspace crates | 2 | Both used (skill-vetter, robot-kit) |

**Validation:**
- `cargo check` ✓
- `cargo check --features rag-pdf` ✓
- `cargo test --lib` - 3818 tests passed ✓

---

*This plan is part of the TopClaw simplification initiative.*
