//! hugr-core — EARLY SKETCH, not meant to compile or be complete.
//!
//! Purpose: make concrete *what the brain actually does*. Read top to bottom.
//! The whole point: the brain is SMALL. It is a reducer + a turn policy +
//! bookkeeping. It does NOT do IO, render, run tools, call HTTP, decide
//! permissions, or schedule. It decides "given everything so far and what just
//! happened, what should happen next?" and hands that decision to the host.
//!
//! Mental model:
//!     poll()   -> drain the commands the brain wants performed
//!     submit() -> feed one event in; the brain folds it into state and may
//!                 queue new commands
//!
//! Everything below uses placeholder types (Value, OpId, etc.) to stay readable.

// ============================================================================
// 0. Primitives
// ============================================================================

pub type OpId = u64;
pub type Value = serde_json::Value; // opaque payload the brain does NOT interpret
pub type Timestamp = u64; // injected, never read from a clock

/// A logical model role, NOT a concrete endpoint. The host resolves it.
/// This is how multi-model works: the brain names a role; the host maps it.
#[derive(Clone)]
pub enum ModelSelector {
    Named(String), // "router" | "big" | "fast" | "summarizer" | "vision" | ...
}

// ============================================================================
// 1. The wire: Commands (brain -> host) and Events (host -> brain)
//    #[non_exhaustive] everywhere so we can add variants without breaking hosts.
// ============================================================================

#[non_exhaustive]
pub enum Command {
    /// Call a model. `model` is a logical selector the host resolves.
    /// The brain reasons about the RESULT (assembles output, extracts tool
    /// calls), which is exactly why this is typed and separate from
    /// StartCapability (see ARCHITECTURE §5.3 / §13).
    StartModelCall { op: OpId, model: ModelSelector, request: ModelRequest },

    /// Invoke a host capability (a tool). `args` is OPAQUE: the brain only
    /// routes it. Adding new tools never touches the core.
    StartCapability { op: OpId, name: String, args: Value },

    /// Spawn a sub-agent (another brain). Seeded by forking the log.
    StartAgent { op: OpId, config: Value, seed: AgentSeed },

    AskUser { op: OpId, prompt: Value },
    RequestPermission { op: OpId, request: PermissionRequest },

    Cancel { op: OpId },

    /// Cosmetic / observability for front-ends. Never affects state.
    Emit(OutputEvent),

    /// Tell the host to durably persist the log up to now.
    Checkpoint,

    Done { reason: Value },
}

#[non_exhaustive]
pub enum Event {
    // Conversational input. Can arrive AT ANY TIME, including mid-turn — the
    // reducer consults `mode` (§4.6). `content` is opaque/rich (text, images...).
    UserInput { content: Value, mode: SteerMode },
    // Pure control signal: cancel current activity, no new content (e.g. ESC).
    UserAbort,

    // --- model streaming -----------------------------------------------------
    // Deltas are TRANSPORT ONLY. They drive live UI via Emit and accumulate in
    // the op buffer. They are NOT persisted individually (see trace size, §12).
    ModelDelta { op: OpId, delta: ModelDelta },
    // The authoritative, consolidated result. This is what the brain LOGIC keys
    // off, and what gets written to the log as a single record. In replay, the
    // host feeds ONLY this (no deltas) and the brain behaves identically.
    ModelDone { op: OpId, output: ModelOutput, usage: Usage },
    ModelError { op: OpId, error: Value },

    // --- capability results --------------------------------------------------
    CapabilityChunk { op: OpId, chunk: Value }, // e.g. a line of stdout (transport)
    // `version` is the standard concurrency envelope (§7.3): present when the op
    // read/refreshed a versioned object, so the brain can update its read-set.
    CapabilityDone { op: OpId, result: Value, version: Option<VersionRef> },
    // `conflict` is set when the host's atomic CAS rejected a stale mutation.
    CapabilityError { op: OpId, error: Value, conflict: Option<VersionRef> },

    // --- sub-agents ----------------------------------------------------------
    AgentDone { op: OpId, result: Value },
    AgentError { op: OpId, error: Value },

    UserAnswer { op: OpId, answer: Value },
    PermissionDecision { op: OpId, decision: Decision },
    OpCancelled { op: OpId },

    // --- injected nondeterminism --------------------------------------------
    Tick { now: Timestamp },
}

// Typed because the brain assembles/inspects them (tool-call detection).
#[non_exhaustive]
pub enum ModelDelta {
    Text(String),
    Reasoning(String),
    ToolCallStart { id: String, name: String },
    ToolCallArgsDelta { id: String, json_fragment: String },
    ToolCallEnd { id: String },
}

/// The consolidated model output the brain reasons about.
pub struct ModelOutput {
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>, // <-- the brain branches on THIS
    pub stop: StopReason,
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value, // opaque to the brain; forwarded to the capability
}

pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

pub enum Decision {
    Allow,
    Deny { reason: String },
}

pub enum AgentSeed {
    Fresh,
    ForkAt { seq: u64 },
    ForkFull,
}

/// How conversational input is handled when it arrives mid-turn (§4.6).
pub enum SteerMode {
    Queue,             // default: process at the next turn boundary
    Interrupt,         // cancel in-flight ops + start a fresh turn now
    AppendAndContinue, // add to context; current op finishes, next call sees it
}

/// Optimistic-concurrency envelope for stateful capabilities (files, PRs, rows).
/// Values are OPAQUE to the brain — it uses only Eq/Hash, never parses them.
/// A read-like capability returns this in a standard slot; the brain records it.
pub struct VersionRef {
    pub object: ObjectKey,
    pub version: Version,
}
pub type ObjectKey = String; // host-canonicalized identity, e.g. abs path / "pr:org/repo#42"
pub type Version = String; // opaque: content hash / etag / git sha / row xmin / ...

// Placeholders for types defined elsewhere (model layer, projection, etc.)
pub struct ModelRequest; // built by the ContextPolicy
pub struct Usage;
pub struct OutputEvent;
pub struct PermissionRequest;

// ============================================================================
// 2. Durable state: the log is the truth; BrainState is derived
// ============================================================================

pub struct LogEntry {
    pub seq: u64,
    pub at: Timestamp,
    pub record: Record,
}

/// What we persist. NOTE: one record per *logical* thing, never per delta.
#[non_exhaustive]
pub enum Record {
    UserMessage { text: String },
    ModelOutput { op: OpId, output_ref: Value /* inline or BlobRef */ },
    ToolResult { op: OpId, name: String, result_ref: Value },
    OpEnded { op: OpId, outcome: OpOutcome, meta: OpMeta },
    // ... permission decisions, agent results, etc.
}

pub enum OpOutcome {
    Ok,
    Error(Value),
    Cancelled { partial: Value },
}

/// Per-op metadata recorded when an op ends. Cost is just ONE field here —
/// timing matters at least as much (latency analysis, "what was slow?",
/// scheduling). Kept open-ended so hosts/policies can attach more without
/// touching the core (narrow-waist: the brain stores, never interprets, `extra`).
pub struct OpMeta {
    pub started_at: Timestamp,
    pub ended_at: Timestamp, // ended_at - started_at = wall-clock latency
    pub model: Option<ModelSelector>, // which logical model (for model ops)
    pub usage: Option<Usage>, // tokens / cost, when applicable
    pub extra: Value, // opaque: provider request-id, cache-hit info, retries, ...
}

pub struct BrainState {
    log: Vec<LogEntry>,
    next_seq: u64,
    next_op: OpId,
    inflight: std::collections::HashMap<OpId, OpState>,
    outbox: Vec<Command>, // commands waiting to be drained by poll()
    now: Timestamp,
    /// Generic optimistic-concurrency read-set (ARCHITECTURE §7.3): last-seen
    /// version per object, folded from capability results. The brain stamps
    /// `expected_version` onto mutations from this; the HOST does the atomic
    /// check. Keys/values are opaque (Eq/Hash only).
    versions: std::collections::HashMap<ObjectKey, Version>,
}

/// Live, in-flight scratch space. Rebuildable by folding the log.
pub enum OpState {
    /// Accumulates model deltas for live UI; the consolidated ModelDone is
    /// authoritative for logic. Holds which selector was used so we can attribute.
    Model { selector: ModelSelector, text_so_far: String },
    Capability { name: String },
    Agent,
    AwaitingUser,
    AwaitingPermission,
}

// ============================================================================
// 3. The brain: poll() + submit() + the reducer
// ============================================================================

pub struct Brain {
    state: BrainState,
    policy: Box<dyn TurnPolicy>, // pluggable: decides model selection, when done
}

impl Brain {
    /// Host drains the commands the brain wants performed. Pure, instant.
    pub fn poll(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.state.outbox)
    }

    /// Host feeds one event. Pure, instant, no IO. THE single entry point for
    /// all of the brain's logic.
    pub fn submit(&mut self, event: Event) {
        match event {
            // ---- 1. user input -> behavior depends on mode + whether busy (§4.6) ----
            Event::UserInput { content, mode } => {
                self.append(Record::UserMessage { text: stringify(&content) });
                match (mode, self.busy()) {
                    // idle, or explicitly queued: just (re)start a turn at the boundary
                    (SteerMode::Queue, _) | (_, false) => self.maybe_resume_model_turn(),
                    // steer now: cancel in-flight ops; the partial work is logged (§6.4)
                    // and becomes context for the fresh turn we kick off on OpCancelled.
                    (SteerMode::Interrupt, true) => self.cancel_all_inflight(),
                    // let the current op finish; its completion will resume the turn
                    (SteerMode::AppendAndContinue, true) => {}
                }
            }
            Event::UserAbort => self.cancel_all_inflight(),

            // ---- 2. model streaming: deltas are cosmetic ----
            Event::ModelDelta { op, delta } => {
                if let Some(OpState::Model { text_so_far, .. }) = self.state.inflight.get_mut(&op) {
                    if let ModelDelta::Text(t) = &delta {
                        text_so_far.push_str(t); // cheap append, for live UI only
                    }
                }
                // forward to front-ends; does NOT touch the log
                self.emit_delta(op, delta);
            }

            // ---- 3. model finished: THIS is where the agentic logic lives ----
            Event::ModelDone { op, output, usage } => {
                self.account(op, usage);
                self.append(Record::ModelOutput { op, output_ref: Value::Null /* or blob */ });
                self.end_op(op, OpOutcome::Ok);

                if output.tool_calls.is_empty() {
                    // model produced a final answer -> the turn is over
                    self.checkpoint();
                    self.done(/* end turn */);
                } else {
                    // model wants tools -> turn each tool call into an op.
                    // The brain ROUTES; it does not interpret the args.
                    for call in output.tool_calls {
                        let cap_op = self.new_op();
                        // permission is its own op; the brain asks, host's policy decides
                        if self.policy.needs_permission(&call.name) {
                            self.push(Command::RequestPermission {
                                op: cap_op,
                                request: PermissionRequest, /* describes `call` */
                            });
                            self.mark(cap_op, OpState::AwaitingPermission);
                        } else {
                            self.start_capability(cap_op, call.name, call.args);
                        }
                    }
                }
            }

            // ---- 4. permission came back ----
            Event::PermissionDecision { op, decision } => match decision {
                Decision::Allow => {
                    // (the brain stashed the pending call; elided here for brevity)
                    // self.start_capability(op, name, args);
                }
                Decision::Deny { reason } => {
                    // feed the denial back to the model as a tool result on next turn
                    self.append(Record::ToolResult {
                        op,
                        name: "<denied>".into(),
                        result_ref: Value::String(reason),
                    });
                    self.maybe_resume_model_turn();
                }
            },

            // ---- 5. a tool finished -> record result, maybe call model again ----
            Event::CapabilityChunk { op, chunk } => self.emit_chunk(op, chunk), // cosmetic
            Event::CapabilityDone { op, result, version } => {
                // Update the optimistic-concurrency read-set (§7.3) if this op
                // read/refreshed a versioned object. Opaque store, no parsing.
                if let Some(v) = version {
                    self.state.versions.insert(v.object, v.version);
                }
                self.append(Record::ToolResult { op, name: self.name_of(op), result_ref: result });
                self.end_op(op, OpOutcome::Ok);
                // once ALL tool ops from this turn are done, call the model again
                self.maybe_resume_model_turn();
            }
            Event::CapabilityError { op, error, conflict } => {
                // A stale-edit conflict (§7.3) is just an error result fed back to
                // the model. We refresh the read-set with the host's current
                // version so the model's *next* edit is stamped correctly.
                if let Some(v) = conflict {
                    self.state.versions.insert(v.object, v.version);
                }
                self.append(Record::ToolResult { op, name: self.name_of(op), result_ref: error });
                self.end_op(op, OpOutcome::Error(Value::Null));
                self.maybe_resume_model_turn();
            }

            // ---- 6. sub-agent finished -> treat result like a tool result ----
            Event::AgentDone { op, result } => {
                self.append(Record::ToolResult { op, name: "<agent>".into(), result_ref: result });
                self.end_op(op, OpOutcome::Ok);
                self.maybe_resume_model_turn();
            }

            // ---- 7. bookkeeping / lifecycle ----
            Event::OpCancelled { op } => self.end_op(op, OpOutcome::Cancelled { partial: Value::Null }),
            Event::Tick { now } => self.state.now = now,

            // ... ModelError, AgentError, UserAnswer handled similarly ...
            _ => {}
        }
    }
}

// ----------------------------------------------------------------------------
// 3b. Small private helpers — note how little "logic" there really is.
// ----------------------------------------------------------------------------

impl Brain {
    /// Begin a model turn: ask the policy which model + assemble context, emit call.
    fn start_model_turn(&mut self) {
        // Compaction is a model op, not a function (§3.4). If projecting would
        // blow the budget, fire a "summarizer" op FIRST; its summary Record lands
        // in the log and the *next* projection evicts the originals to references.
        if self.policy.needs_compaction(&self.state) {
            let op = self.new_op();
            self.mark(op, OpState::Model { selector: ModelSelector::Named("summarizer".into()), text_so_far: String::new() });
            self.push(Command::StartModelCall { op, model: ModelSelector::Named("summarizer".into()), request: self.policy.compaction_request(&self.state.log) });
            return; // resume the real turn once the summary arrives
        }
        let op = self.new_op();
        let selector = self.policy.choose_model(&self.state); // multi-model decision
        // Projection is PURE/SYNC: reads the log (incl. existing summaries), sums
        // host-provided token counts (§3.5), emits a ModelRequest. Never calls a model.
        let request = self.policy.project_context(&self.state.log);
        self.mark(op, OpState::Model { selector: selector.clone(), text_so_far: String::new() });
        self.push(Command::StartModelCall { op, model: selector, request });
    }

    /// After all tool ops of a turn resolve, feed results back to the model.
    fn maybe_resume_model_turn(&mut self) {
        if self.no_tool_ops_inflight() {
            self.start_model_turn();
        }
    }

    fn start_capability(&mut self, op: OpId, name: String, mut args: Value) {
        // If this capability's schema declares it mutates a versioned object,
        // pluck the declared object-key field (generic, NOT capability-specific
        // logic) and stamp the last-seen version as `expected_version`. The model
        // never sees the token; the HOST does the atomic check (§7.3).
        if let Some(object) = self.declared_object_key(&name, &args) {
            if let Some(version) = self.state.versions.get(&object) {
                set_expected_version(&mut args, version); // writes into the opaque args blob
            }
        }
        self.mark(op, OpState::Capability { name: name.clone() });
        self.push(Command::StartCapability { op, name, args });
    }

    // trivially small bookkeeping ------------------------------------------------
    fn new_op(&mut self) -> OpId { let id = self.state.next_op; self.state.next_op += 1; id }
    fn push(&mut self, c: Command) { self.state.outbox.push(c); }
    fn mark(&mut self, op: OpId, s: OpState) { self.state.inflight.insert(op, s); }
    fn end_op(&mut self, op: OpId, outcome: OpOutcome) {
        // Assemble per-op metadata: timing (from the op's recorded start time to
        // `now`), the model selector, usage, etc. Stored on every OpEnded so the
        // log carries latency + cost for free — queryable later without a side table.
        let meta = self.op_meta(op);
        self.state.inflight.remove(&op);
        self.append(Record::OpEnded { op, outcome, meta });
    }
    fn append(&mut self, record: Record) {
        let seq = self.state.next_seq; self.state.next_seq += 1;
        self.state.log.push(LogEntry { seq, at: self.state.now, record });
    }
    fn checkpoint(&mut self) { self.push(Command::Checkpoint); }
    fn done(&mut self) { self.push(Command::Done { reason: Value::Null }); }
    fn account(&mut self, _op: OpId, _usage: Usage) {/* stash usage onto the op for op_meta */}
    fn op_meta(&self, _op: OpId) -> OpMeta { unimplemented!("build from op start time + now + usage") }
    fn emit_delta(&mut self, _op: OpId, _d: ModelDelta) { /* push Command::Emit */ }
    fn emit_chunk(&mut self, _op: OpId, _c: Value) { /* push Command::Emit */ }
    fn name_of(&self, _op: OpId) -> String { String::new() }
    fn no_tool_ops_inflight(&self) -> bool { true }
    fn busy(&self) -> bool { !self.state.inflight.is_empty() }
    fn cancel_all_inflight(&mut self) {
        // emit Cancel for each in-flight op; the fresh turn is kicked off when the
        // resulting OpCancelled events drain (so partial work is logged first).
        let ops: Vec<OpId> = self.state.inflight.keys().copied().collect();
        for op in ops { self.push(Command::Cancel { op }); }
    }
    /// Generic: read the object-key field DECLARED by the capability's schema
    /// (e.g. "the `path` arg"). Not capability-specific logic — just a field pluck.
    /// Returns None for stateless capabilities that declare no versioned object.
    fn declared_object_key(&self, _name: &str, _args: &Value) -> Option<ObjectKey> { None }
}

// Generic helper: write `expected_version` into the (opaque) args blob at the
// location the capability's schema declares. Lives outside the brain's branching.
fn set_expected_version(_args: &mut Value, _version: &Version) {/* args["expected_version"] = ... */}

// ============================================================================
// 4. The pluggable policy — the ONLY place "agent strategy" lives.
//    Swap this to change behavior without touching the reducer.
// ============================================================================

pub trait TurnPolicy {
    /// Multi-model brain: pick which logical model to call for the next step.
    /// Could be static ("always big"), heuristic (size/task based), or call a
    /// "router" model first and decide from its output.
    fn choose_model(&self, state: &BrainState) -> ModelSelector;

    /// Render the model context from the log. PURE/SYNC: include / summarize /
    /// evict-to-reference / drop, summing host-provided token counts (§3.5).
    /// Reads existing summaries; never calls a model.
    fn project_context(&self, log: &[LogEntry]) -> ModelRequest;

    /// Whether the log has grown past the budget watermark and should be compacted
    /// (§3.4). If true, the brain fires a "summarizer" op before the real turn.
    fn needs_compaction(&self, state: &BrainState) -> bool;
    /// Build the request for the summarizer op (which span to compact).
    fn compaction_request(&self, log: &[LogEntry]) -> ModelRequest;

    /// Whether a given capability requires a permission round-trip.
    fn needs_permission(&self, capability: &str) -> bool;
}

// Generic helper: render opaque user content to text for the log record.
fn stringify(_content: &Value) -> String { String::new() }

// ============================================================================
// SUMMARY — what the brain does, exhaustively:
//   1. Maintain the append-only log + the in-flight op table (bookkeeping).
//   2. Run the turn loop: user -> model -> (tool calls?) -> tools -> model ...
//   3. Ask a pluggable TurnPolicy: which model, how to project context,
//      whether permission is needed.
//   4. Route opaque capability args/results between model and host.
//   5. Keep the optimistic-concurrency read-set and stamp `expected_version`
//      onto mutations (§7.3). The brain tracks versions; the HOST does the
//      atomic check.
//   6. Emit permission requests (host decides) and cosmetic UI events.
//   7. Decide when a turn/session is Done; emit Checkpoint.
// That is ALL. Everything hard (IO, HTTP, rendering, scheduling, policy
// decisions, model resolution, the atomic CAS check) lives in the host.
// ============================================================================
