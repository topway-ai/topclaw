//! Placeholder live test for the legacy quota-tools surface.
//!
//! The old `topclaw::tools::quota_tools` module no longer exists in the current
//! codebase, so the historical live tests that imported those tool types would
//! fail the entire test-target compile even when unrelated tests were selected.
//!
//! Keep this file as an explicit ignored placeholder until a new public quota
//! command or tool surface is introduced and covered with real live tests.

#[test]
#[ignore = "legacy quota tools were removed; replace with live coverage for the current quota surface before re-enabling"]
fn quota_tools_live_is_deferred_until_new_public_surface_exists() {}
