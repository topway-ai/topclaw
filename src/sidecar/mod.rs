//! Computer-use sidecar — built-in reference implementation.
//!
//! Serves the protocol defined in `docs/computer-use-sidecar-protocol.md` on a
//! local axum HTTP server, backed by Linux desktop utilities (`xdotool`,
//! `wmctrl`, `scrot`, `xdg-open`, `pkill`). The agent launches this via the
//! approval-gated `computer_use_sidecar_start` tool (see
//! `src/tools/computer_use_sidecar_start.rs`), so a Telegram user can enable
//! desktop automation with a single Approve tap.
//!
//! # Safety
//!
//! - Linux-only at runtime. Other OSes return a compile-time-friendly error
//!   from `run_server` because the action handlers shell out to Linux-specific
//!   binaries.
//! - Every request is policy-gated: `window_allowlist` substring match,
//!   coordinate clamps, `allowed_domains` for `open`.
//! - Optional Bearer auth via the `TOPCLAW_SIDECAR_API_KEY` env var. When set,
//!   requests without a matching header are rejected before any action runs.
//! - Binds to the address supplied on the CLI. The auto-spawn tool defaults to
//!   `127.0.0.1`; non-localhost binds are allowed but should be rare.

pub mod linux;
pub mod server;

pub use server::run_server;
