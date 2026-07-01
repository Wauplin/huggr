//! # hugr-providers — model adapters
//!
//! Provider adapters live at the edge (ARCHITECTURE §5.2): they translate the
//! canonical [`ModelRequest`](hugr_core::ModelRequest) /
//! [`ModelOutput`](hugr_core::ModelOutput) to/from a concrete provider's wire
//! format, streaming deltas back through a [`ModelSink`](hugr_host::ModelSink).
//!
//! Phase 1 ships one adapter: [`OpenAiAdapter`] (chat completions, streaming).

mod openai;

pub use openai::OpenAiAdapter;
