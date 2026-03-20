//! Agent orchestration surfaces.
//!
//! This module contains the main LLM-to-tool loop plus supporting pieces for
//! prompt construction, tool-call parsing, routing, memory loading, and the
//! optional research phase.
//!
//! Use [`Agent`] when you want a reusable in-process agent instance. Use
//! [`run`] or [`process_message`] when you want the standard config-driven
//! runtime path used by the CLI and channels.
#[allow(clippy::module_inception)]
pub mod agent;
pub mod classifier;
pub mod dispatcher;
pub mod loop_;
pub mod memory_loader;
pub mod prompt;
pub mod research;
pub(crate) mod wiring;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use agent::{Agent, AgentBuilder};
#[allow(unused_imports)]
pub use loop_::{process_message, run};
