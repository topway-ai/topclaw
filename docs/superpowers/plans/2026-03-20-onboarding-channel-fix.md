# Onboarding Channel Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make shipped TopClaw binaries include Telegram and Discord in onboarding, and update the onboarding channel picker to show Telegram first, Discord second, and an `Other channels...` entry for the remaining compiled channels.

**Architecture:** Keep the fix local to two surfaces. Update the release workflow feature set so prebuilt binaries compile the intended channels, and add a small onboarding presentation layer that splits recommended channels from the full compiled channel list without changing the underlying channel setup flows.

**Tech Stack:** Rust, GitHub Actions workflow YAML, existing dialoguer-based onboarding UI, cargo test

---

### Task 1: Lock In Regression Coverage

**Files:**
- Modify: `src/onboard/wizard.rs`
- Test: `src/onboard/wizard.rs`

- [ ] **Step 1: Write the failing test**

Add focused onboarding regression tests that assert:
- default onboarding starts on `Telegram`
- the top-level channel menu exposes `Telegram`, `Discord`, and `Other channels...` in that order when compiled features allow
- the secondary menu includes advanced channels like `Webhook` and `Lark/Feishu`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test onboarding_channel -- --nocapture`
Expected: FAIL because the current menu is flat and has no `Other channels...` grouping.

- [ ] **Step 3: Write minimal implementation scaffolding**

Add the smallest helper API needed to support grouped channel menus and keep existing channel setup handlers unchanged.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test onboarding_channel -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/onboard/wizard.rs src/onboard/wizard_channels.rs
git commit -m "fix: group onboarding channel choices"
```

### Task 2: Implement the Onboarding Menu Split

**Files:**
- Modify: `src/onboard/wizard.rs`
- Modify: `src/onboard/wizard_channels.rs`
- Test: `src/onboard/wizard.rs`

- [ ] **Step 1: Implement grouped menu choice helpers**

Add a top-level choice for `Other channels...`, preserve `Done`, and build a second menu containing the remaining compiled channels.

- [ ] **Step 2: Keep selection defaults intuitive**

Ensure the initial menu defaults to `Telegram` when no channels are configured and to `Done` once a channel is configured.

- [ ] **Step 3: Reuse existing setup flows**

Route `Telegram` and `Discord` directly from the top-level menu, and route advanced channels through the secondary menu back into the existing `setup_*_channel` functions.

- [ ] **Step 4: Run targeted onboarding tests**

Run: `cargo test channel_menu -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/onboard/wizard.rs src/onboard/wizard_channels.rs
git commit -m "fix: prioritize recommended onboarding channels"
```

### Task 3: Restore Release Packaging Defaults

**Files:**
- Modify: `.github/workflows/pub-release.yml`

- [ ] **Step 1: Update release feature selection**

Replace the release feature list so prebuilt artifacts include Telegram and Discord alongside the current Lark/web-fetch coverage.

- [ ] **Step 2: Verify workflow syntax context**

Read the surrounding workflow block to confirm the feature string is consumed by both `cargo build` and `cross build` release jobs.

- [ ] **Step 3: Run targeted verification**

Run: `cargo test onboarding_channel channel_menu -- --nocapture`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/pub-release.yml src/onboard/wizard.rs src/onboard/wizard_channels.rs
git commit -m "fix: restore recommended channels in release builds"
```
