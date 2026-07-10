//! # hugr-providers — model adapters
//!
//! Provider adapters translate the canonical [`ModelRequest`](hugr_core::ModelRequest) / [`ModelOutput`](hugr_core::ModelOutput) to/from a concrete provider's wire format, streaming deltas back through a [`ModelSink`](hugr_host::ModelSink).

mod openai;

pub use openai::OpenAiAdapter;
