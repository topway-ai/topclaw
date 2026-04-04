//! Hardware discovery and peripheral integration for TopClaw.
//!
//! This crate provides USB device enumeration, board introspection,
//! and peripheral tool integration (STM32, RPi GPIO, Arduino, etc.).
//!
//! See `docs/hardware-peripherals-design.md` for the full design.
//!
//! **Status:** Source extracted from the main `topclaw` crate. Internal
//! `crate::` imports still reference parent-crate types (`Config`, `Tool`,
//! `ToolResult`, `HardwareCommands`). Before this crate compiles
//! independently, a shared `topclaw-traits` crate must be introduced
//! to break the circular dependency.
//!
//! The source modules below are preserved but not yet re-exported.
//! To compile, run: `cargo build -p topclaw-hardware` (will fail until
//! trait extraction is complete).
