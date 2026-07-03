# hugr-docs-2 — a subagent as a definition folder

This folder is what `crates/hugr-docs` becomes after the toolkit exists (ROADMAP T0–T2): the **same docs-retrieval subagent, expressed entirely as data**. It is an illustrative example — nothing here runs today. Its purpose is to show what "building a new subagent" will feel like.

The entire agent is two files:

```
hugr-docs-2/
  hugr.toml     # the manifest: identity, model tiers + pricing, tool grants + scopes, limits, traces
  SYSTEM.md     # the system prompt (markdown, a few template vars)
```

There is no `src/`, no `Cargo.toml`, no `pyproject.toml`, no runner, no JSON post-processing code. Compare with `crates/hugr-docs`: its ~2000 lines of Rust split into (a) seven bespoke read-only tools → now the library `fs_read` grant, (b) engine wiring and env-var plumbing → now the `[models]` section, (c) answer assembly, cost math, and error-as-JSON handling → now the universal `Answer` contract that `hugr-agent` provides to every subagent, (d) the CLI and the PyO3 binding → now generated surfaces. What remains agent-specific is exactly the manifest and the prompt — which is the thesis: **a subagent is a system prompt plus tools with privileges; everything else is shared infrastructure.**

## The development loop (T1)

```bash
hugr new my-docs-agent --template docs        # scaffold a folder like this one
hugr run . "Which repositories do I watch by default?" --json   # interpret the definition, no build
```

`hugr run` parses `hugr.toml`, registers exactly the granted tools (jailed to their declared scopes), assembles the `hugr-agent` runtime, executes one ask, persists the trace, and prints the standard answer:

```json
{
  "status": "success",
  "message": "By default, you'll be watching all the organizations you are a member of...",
  "trace_id": "tr_01hx3k9f2m",
  "blobs": [],
  "metadata": {
    "duration_ms": 4210,
    "cost_micro_usd": 1300,
    "tokens_in": 1000,
    "tokens_out": 200,
    "model_calls": 2,
    "tool_calls": 3,
    "per_tier": [{ "tier": "medium", "calls": 2, "tokens_in": 1000, "tokens_out": 200, "cost_micro_usd": 1300 }]
  },
  "extra": { "related_documents": ["hub/notifications.md"] }
}
```

Every field above except `extra` is the fixed contract shared by all subagents (ARCHITECTURE §18.1). `extra` carries this agent's `related_documents`, declared and schema-checked by the manifest's `[answer.extra_schema]`. Errors are answers too — a bad API key, a missing docs root, or a blown `[limits]` budget all come back as `"status": "error"` with exit code 0, so callers branch on data.

## Resume and fork (the orchestration contract)

```bash
hugr run . "Which repositories do I watch by default?"                 # → trace_id: tr_A
hugr run . "And how do I mute one of them?" --trace tr_A               # resumes tr_A → tr_B (depends_on: tr_A)
hugr run . "What about for datasets instead of models?" --trace tr_A   # forks tr_A  → tr_C (sibling of tr_B)
hugr traces .                                                          # lineage tree: tr_A → { tr_B, tr_C }
hugr replay . tr_B --step                                              # deterministic replay of any stored trace
```

Traces are immutable: a follow-up never mutates its parent, so an orchestrator can fan out sibling explorations from any past point without growing one shared context, and each answer's metadata tells it exactly what that branch cost.

## Shipping it (T2)

Surface choice is a build-time flag, never part of this folder:

```bash
hugr build . --surface cli,python,mcp
```

- **CLI** — a standalone `hugr-docs` binary with the universal shape: `hugr-docs "question" [--trace <id>] [--json|--pretty] [--describe] [--traces] [--config]`. `--describe` prints the agent card (tools + privileges, tiers, pricing, limits) straight from the manifest; `--config` prints effective config with per-key provenance and redacted secrets.
- **Python** — a wheel exposing `hugr_docs.answer(question, trace_id=None, docs_path=None, **overrides) -> dict` (never raises for run failures; each config key falls back to its env var) plus `describe()` / `traces()` — the current binding's API, now generated instead of hand-written.
- **MCP** — `hugr-docs --mcp-serve` exposes one `ask` tool over stdio; `trace_id` rides the tool arguments, so any MCP-speaking orchestrator (e.g. Claude Code) gets resume/fork and cost metadata for free.

```python
import hugr_docs

first = hugr_docs.answer("Which repositories do I watch by default?", docs_path="./archive-light-2026-07-01")
follow = hugr_docs.answer("And how do I mute one?", trace_id=first["trace_id"])
print(follow["message"], follow["metadata"]["cost_micro_usd"])
```

## Why this is auditable and safe

Read `hugr.toml` top to bottom and you know everything this agent can ever do: read files under one root, take notes in its own scratchpad, call one model endpoint, spend at most 5 cents per ask. `shell`, `fs_write`, and `http_fetch` are not granted, so the built binary contains no code path to them — the sandbox is what gets registered, not a policy saying "no" at runtime.
