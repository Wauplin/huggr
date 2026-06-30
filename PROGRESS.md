# Progress

Running log of what's implemented, phase by phase (see `docs/ROADMAP.md`).

## Phase 0 — Pure core skeleton (no IO) ✅

**Goal:** the brain exists as a pure state machine with zero IO.

Done:

- Workspace set up (`crates/baton-core`), ready to grow into the full layout.
- `baton-core` — the sans-IO reducer, split into modules:
  - `primitives.rs` — `OpId`, `Seq`, `Timestamp`, `Value`, `ObjectKey`.
  - `model.rs` — canonical `ModelRequest`/`ModelDelta`/`ModelOutput`, `ToolCall`,
    `ToolSchema`, `Usage`, `ModelSelector` (+ constructors).
  - `command.rs` / `event.rs` — the two-enum brain↔host contract,
    `#[non_exhaustive]` throughout.
  - `record.rs` — the append-only log (`LogEntry`, `Record`, `OpOutcome`, `OpMeta`).
  - `state.rs` — `BrainState` + in-flight op table (derived; foldable from the log).
  - `policy.rs` — pluggable `TurnPolicy` + `StaticPolicy` (trivial pass-through projection).
  - `brain.rs` — `Brain::poll()` / `submit()` + the turn-loop reducer.
- Tests (`crates/baton-core/tests`): scripted session, permission round-trip,
  parallel tool calls, projection contents, deterministic replay, delta-vs-log,
  JSON round-trip. **9 passing.**

**Exit criteria — met:**

- ✅ Scripted `user → model → tool → model → done` reduces to the expected
  command sequence (`scripted_session.rs`).
- ✅ Deterministic replay: same event stream twice → identical commands
  (`determinism.rs`).
- ✅ No `tokio`/`reqwest`/`fs` in `baton-core` (`cargo tree -p baton-core` shows
  only `serde`/`serde_json`).

Decisions:

- Single crate for Phase 0; model types kept in `baton-core` (move to
  `baton-model` later if needed).
- `#[non_exhaustive]` on enums **and** host-facing structs, with constructors on
  the structs (forward-compatible, narrow-waist).
- Dropped `panic = "abort"` from the release profile (conflicts with the test
  harness; belongs in a WASM-specific profile in Phase 4).

## Phase 1 — Batteries-included CLI host (the showcase) ✅

**Goal:** a real, usable terminal agent driven by the Phase 0 core.

Done:

- `baton-host`: the tokio [`Engine`] driver loop (drain `poll()` → perform
  commands as concurrent tasks → await next event → `submit()`), plus:
  - [`Capability`] + [`ModelAdapter`] traits and their registries.
  - Host-side permission [`Policy`]: `AllowAll`, `DenyAll`, `Interactive` (prompts).
  - [`Frontend`] trait + streaming `StdoutFrontend`.
  - `EngineBuilder` that assembles the brain's `StaticPolicy` from registered
    capabilities (their schemas → advertised tools; sensitive ones → gated set).
- Capabilities (`baton-host::capabilities`): `shell` (streams stdout),
  `fs_read` (read-only, no permission), `fs_write`, `http`.
- `baton-providers`: `OpenAiAdapter` — chat completions with streaming SSE,
  tool-call assembly, usage accounting, configurable base URL. Defaults target
  the **Hugging Face router** (`https://router.huggingface.co/v1`,
  `meta-llama/Llama-3.3-70B-Instruct`); the API key resolves from
  `OPENAI_API_KEY` → `HF_TOKEN` → `hf auth token`.
- `baton-cli`: the `baton` binary. One-shot (`baton "prompt"`) or interactive
  REPL; `-y/--yes` for allow-all.

Refinement to `baton-core` made for real providers: the durable `ToolResult`
now carries the originating model `tool_call` id, so projection emits provider-
correct `tool_call_id` correlation. Added `ModelOutput::new`, `ModelRequest::new`
and `SamplingParams` builders (host-facing structs are `#[non_exhaustive]`).

Tests (17 total across the workspace):

- `baton-host/tests/end_to_end.rs` — a real multi-turn session driven through
  the tokio loop with a scripted model + the **real shell capability**; plus a
  denied-permission round-trip.
- `baton-providers` — unit tests for request building + SSE accumulation, and
  `tests/streaming.rs` driving the adapter against a **local mock SSE server**
  (real reqwest streaming path).

**Exit criteria:**

- ✅ "CLI on a laptop" host setup ≈ 10 lines on top of `baton-host` (see the
  marked block in `crates/baton-cli/src/main.rs`).
- ✅ Genuine multi-turn session end-to-end. Verified **live** against the HF
  router: `baton -y "Use the shell tool to run 'echo baton-live-test', then
  tell me what it printed."` — the model called the shell tool, the host ran it
  and streamed the output, and the model produced a final answer. Also covered
  by the driver-loop + mock-SSE tests for CI (no key needed).

[`Engine`]: crates/baton-host/src/engine.rs
[`Capability`]: crates/baton-host/src/capability.rs
[`ModelAdapter`]: crates/baton-host/src/model.rs
[`Policy`]: crates/baton-host/src/policy.rs
[`Frontend`]: crates/baton-host/src/frontend.rs
