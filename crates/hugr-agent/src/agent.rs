//! The first real `Agent::ask` path — resume & fork semantics
//! (ARCHITECTURE §19.2, ROADMAP T0.3).
//!
//! An [`Agent`] is a reusable configuration (system prompt, model adapters,
//! capabilities, permission policy) plus a [`TraceStore`]. Each
//! [`ask`](Agent::ask) builds a **fresh** engine:
//!
//! - `trace_id: None` → a fresh brain runs the turn and the session persists
//!   as a new **root** trace.
//! - `trace_id: Some(parent)` → the parent trace is loaded from the store and
//!   **re-folded** into the fresh brain via [`EngineBuilder::resume`] — zero IO
//!   beyond the one file read, no model/tool re-calls (ARCHITECTURE §15.1) —
//!   then the new question runs as a live turn and the whole session (old +
//!   new events) persists as a **new** trace with `depends_on = parent`.
//!
//! The parent file is never touched, so forking is just asking the same
//! parent twice: the two children are sibling traces in the store's DAG.
//!
//! Error discipline (§18.1): *run* failures — the model erroring, no final
//! answer — are **answers** (`status: Error`) with a persisted trace, so the
//! caller still gets a `trace_id` to inspect. Only *infrastructure* failures
//! (an unknown parent id, a store write error) return [`AskError`]; surfaces
//! convert those to error answers at their own boundary (T0.8).

use std::sync::Arc;
use std::time::Instant;

use hugr_core::{LogEntry, ModelSelector, Record, SamplingParams};
use hugr_host::policy::AllowAll;
use hugr_host::{Capability, Clock, Engine, Frontend, ModelAdapter, Policy};

use crate::contract::{Answer, AnswerMeta, AnswerStatus, Ask, TraceId};
use crate::store::{StoreError, TraceHeader, TraceStore};

/// A configured subagent: ask it questions, get [`Answer`]s, resume or fork
/// any stored trace. Build one with [`Agent::builder`].
///
/// Cheap to share pieces: adapters, capabilities, and the policy are `Arc`s,
/// so each ask assembles a fresh engine without re-constructing them.
#[non_exhaustive]
pub struct Agent {
    name: String,
    version: String,
    store: TraceStore,
    system_prompt: Option<String>,
    models: Vec<(ModelSelector, Arc<dyn ModelAdapter>)>,
    default_model: Option<ModelSelector>,
    capabilities: Vec<Arc<dyn Capability>>,
    policy: Arc<dyn Policy>,
    sampling: Option<SamplingParams>,
    clock: Option<Clock>,
}

impl Agent {
    /// Start building an agent. `name`/`version` are stamped into every trace
    /// header; `store` is where the immutable traces live.
    pub fn builder(
        name: impl Into<String>,
        version: impl Into<String>,
        store: TraceStore,
    ) -> AgentBuilder {
        AgentBuilder {
            name: name.into(),
            version: version.into(),
            store,
            system_prompt: None,
            models: Vec::new(),
            default_model: None,
            capabilities: Vec::new(),
            policy: None,
            sampling: None,
            clock: None,
        }
    }

    /// The trace store this agent persists into.
    pub fn store(&self) -> &TraceStore {
        &self.store
    }

    /// Run one ask to completion (ARCHITECTURE §18.1/§19.2). See the module
    /// docs for the fresh-vs-resume split and the error discipline.
    pub async fn ask(&self, ask: Ask) -> Result<Answer, AskError> {
        let started = Instant::now();
        let parent = ask.trace_id.clone();

        // Assemble a fresh engine per ask. Recording is always on: the trace
        // *is* the product here.
        let mut builder = Engine::builder()
            .record(true)
            .policy(self.policy.clone())
            .frontend(Box::new(SilentFrontend));
        for (selector, adapter) in &self.models {
            builder = builder.model(selector.clone(), adapter.clone());
        }
        if let Some(selector) = &self.default_model {
            builder = builder.default_model(selector.clone());
        }
        for capability in &self.capabilities {
            builder = builder.capability(capability.clone());
        }
        if let Some(system) = &self.system_prompt {
            builder = builder.system_prompt(system.clone());
        }
        if let Some(sampling) = &self.sampling {
            builder = builder.sampling(sampling.clone());
        }
        if let Some(clock) = &self.clock {
            builder = builder.clock(clock.clone());
        }
        if let Some(parent_id) = &parent {
            // Load the parent (one file read) and re-fold its recorded events
            // into the fresh brain — no model or tool is ever re-run for work
            // that already happened (§15.1); `resume` only rebuilds state.
            let trace = self.store.get(parent_id)?;
            builder = builder.resume(trace);
        }
        let mut engine = builder.build();

        // Accounting baseline: on resume the brain's log already holds the
        // parent's entries; this ask's meta must cover only the new turn.
        let baseline = engine.brain().state().log().len();

        engine.user_turn(ask.question.clone()).await;
        engine.session_end();

        let log = engine.brain().state().log();
        let (status, message) = final_answer(log);
        let metadata = meta_from_log(&log[baseline..], started.elapsed().as_millis() as u64);

        // Persist old + new as one NEW immutable trace; the parent file is
        // never mutated — lineage lives in `depends_on` (§19.2).
        let trace = engine
            .trace()
            .expect("recording is always enabled on an agent engine");
        let mut header = TraceHeader::new(
            &self.name,
            &self.version,
            &ask.question,
            status_wire(status),
        );
        if let Some(parent_id) = parent {
            header = header.with_depends_on(parent_id);
        }
        let trace_id = self.store.put(trace, header)?;

        Ok(Answer::new(status, message, trace_id, metadata))
    }
}

/// Builds an [`Agent`]. Mirrors `hugr_host::EngineBuilder` for the pieces an
/// agent definition declares; everything not set gets the host default.
#[non_exhaustive]
pub struct AgentBuilder {
    name: String,
    version: String,
    store: TraceStore,
    system_prompt: Option<String>,
    models: Vec<(ModelSelector, Arc<dyn ModelAdapter>)>,
    default_model: Option<ModelSelector>,
    capabilities: Vec<Arc<dyn Capability>>,
    policy: Option<Arc<dyn Policy>>,
    sampling: Option<SamplingParams>,
    clock: Option<Clock>,
}

impl AgentBuilder {
    /// Register a model adapter under a logical selector. The first registered
    /// selector is the default unless [`default_model`](Self::default_model)
    /// overrides it (same rule as the engine builder).
    pub fn model(mut self, selector: ModelSelector, adapter: Arc<dyn ModelAdapter>) -> Self {
        self.models.push((selector, adapter));
        self
    }

    /// Override which logical selector the turn policy calls.
    pub fn default_model(mut self, selector: ModelSelector) -> Self {
        self.default_model = Some(selector);
        self
    }

    /// Grant a capability (tool). Sandbox-by-registration (§18.2): only what
    /// is registered here exists for the agent — never register more than the
    /// definition grants.
    pub fn capability(mut self, capability: Arc<dyn Capability>) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Set the system prompt.
    pub fn system_prompt(mut self, system: impl Into<String>) -> Self {
        self.system_prompt = Some(system.into());
        self
    }

    /// Set the host permission policy (default: `AllowAll` — appropriate for
    /// pre-vetted, jailed tool sets).
    pub fn policy(mut self, policy: Arc<dyn Policy>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Set sampling parameters for every model request.
    pub fn sampling(mut self, sampling: SamplingParams) -> Self {
        self.sampling = Some(sampling);
        self
    }

    /// Override the host clock (tests inject a deterministic counter so
    /// recorded traces are reproducible).
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    pub fn build(self) -> Agent {
        Agent {
            name: self.name,
            version: self.version,
            store: self.store,
            system_prompt: self.system_prompt,
            models: self.models,
            default_model: self.default_model,
            capabilities: self.capabilities,
            policy: self.policy.unwrap_or_else(|| Arc::new(AllowAll)),
            sampling: self.sampling,
            clock: self.clock,
        }
    }
}

/// Infrastructure failures of an ask — everything that prevents a trace from
/// being persisted at all. Run failures are *answers*, not errors (§18.1).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AskError {
    /// The parent trace could not be loaded, or the new trace could not be
    /// persisted.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The parent id an [`AskError::Store`] not-found refers to, if any — a small
/// convenience for surfaces mapping infra errors to error answers.
impl AskError {
    pub fn missing_trace(&self) -> Option<&TraceId> {
        match self {
            AskError::Store(StoreError::NotFound { id }) => Some(id),
            _ => None,
        }
    }
}

/// Extract the final answer from the durable log: the last model output with
/// no tool calls is the turn's answer. No text means the run failed before
/// answering — an error *answer* (§18.1), with the terminal error surfaced.
/// Off-topic classification is agent-specific and lives above this layer
/// (`Answer.extra` / the docs port, T0.8).
fn final_answer(log: &[LogEntry]) -> (AnswerStatus, String) {
    let final_text = log.iter().rev().find_map(|entry| match &entry.record {
        Record::ModelOutput { output, .. } if output.tool_calls.is_empty() => {
            Some(output.text.clone())
        }
        _ => None,
    });
    match final_text {
        Some(text) => (AnswerStatus::Success, text),
        None => (
            AnswerStatus::Error,
            missing_final_answer_message(log).to_string(),
        ),
    }
}

fn missing_final_answer_message(log: &[LogEntry]) -> String {
    let terminal_error = log.iter().rev().find_map(|entry| match &entry.record {
        Record::OpEnded {
            outcome: hugr_core::OpOutcome::Error(error),
            ..
        } => Some(error.to_string()),
        _ => None,
    });
    match terminal_error {
        Some(error) => format!("model did not produce a final answer; last error: {error}"),
        None => "model did not produce a final answer".to_string(),
    }
}

/// Minimal accounting for this ask, folded from the *new* slice of the log
/// (a resumed ask never re-bills its ancestry). Cost stays 0 until per-tier
/// pricing lands (T0.6).
fn meta_from_log(new_entries: &[LogEntry], duration_ms: u64) -> AnswerMeta {
    let mut tokens_in = 0u64;
    let mut tokens_out = 0u64;
    let mut model_calls = 0u32;
    let mut tool_calls = 0u32;
    for entry in new_entries {
        let Record::OpEnded { meta, .. } = &entry.record else {
            continue;
        };
        if let Some(usage) = &meta.usage {
            tokens_in += usage.input_tokens;
            tokens_out += usage.output_tokens;
            model_calls += 1;
        } else if meta.model.is_none() {
            tool_calls += 1;
        }
    }
    let mut meta = AnswerMeta::new()
        .with_duration_ms(duration_ms)
        .with_tool_calls(tool_calls);
    meta.tokens_in = tokens_in;
    meta.tokens_out = tokens_out;
    meta.model_calls = model_calls;
    meta
}

/// The wire string of an [`AnswerStatus`] as stamped into trace headers —
/// matches the contract's serde `snake_case` form.
fn status_wire(status: AnswerStatus) -> &'static str {
    match status {
        AnswerStatus::Success => "success",
        AnswerStatus::OffTopic => "off_topic",
        AnswerStatus::Error => "error",
        // `AnswerStatus` is #[non_exhaustive]; new variants must add a wire
        // string here alongside the contract change.
        #[allow(unreachable_patterns)]
        _ => "error",
    }
}

/// A no-op front-end: a subagent's product is its `Answer` + trace, not a
/// terminal render. Surfaces that want live output can grow a builder knob
/// later without touching the contract.
struct SilentFrontend;

impl Frontend for SilentFrontend {}
