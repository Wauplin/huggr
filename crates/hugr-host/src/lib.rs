//! # hugr-host — the default native host
//!
//! `hugr-host` is the **environment-specific** layer that drives the sans-IO
//! [`hugr_core::Brain`]: it performs all IO, runs concurrency on tokio, and
//! turns the brain's [`Command`](hugr_core::Command)s into real effects
//! (model calls, tool calls), feeding results back as
//! [`Event`](hugr_core::Event)s.
//!
//! What lives here (host concerns, never in the core): the [`Capability`] and
//! [`ModelAdapter`] traits and their registries, the [`Frontend`] trait, the
//! MCP stdio client ([`mcp`]), JSON-line framing ([`framing`]), and the tokio
//! [`Engine`] driver loop. The subagent runtime (`hugr-agent`) builds on this
//! surface; tools live in `hugr-toolkit`'s library.

mod capability;
mod engine;
pub mod framing;
mod frontend;
pub mod mcp;
mod model;

pub use capability::{Capability, CapabilityRegistry, ChunkSink};
pub use engine::{Clock, Engine, EngineBuilder, EventSender, estimate_text_tokens};
pub use frontend::Frontend;
pub use mcp::{McpError, McpServerConfig, McpToolCapability};
pub use model::{ModelAdapter, ModelRegistry, ModelSink};

// Re-export the trace + replay surface so a host embedder needs only one crate
// to record a session and replay it (the persistence crate lives behind us).
pub use hugr_replay::{self, Inspector, Replay, Step, Trace, TraceError};
