//! Long-running goal execution primitives.
//!
//! This subsystem is intentionally small at the module surface today. The main
//! implementation lives in [`engine`], which tracks goals, steps, status, and
//! priority for higher-level autonomous workflows.
pub mod engine;
