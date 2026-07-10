# An agent entirely in Python

This tutorial defines a Hugr subagent from scratch in pure Python. The system prompt is a string, model config is a dict, and tools are ordinary callables. The agent runs on the same Rust runtime as every other surface.

The tutorial covers the `hugr-agents` package end to end. Topics include the `@hugr.tool` decorator for sync and async tools, the `Agent` constructor and its manifest-shaped config, `agent.ask()` for blocking runs, and `async for event in agent.run(...)` for streaming. It also covers generating tool and response JSON Schemas from Pydantic dataclasses, `agent.feedback()`, `agent.stats()`, and Rust CLI verification of traces stored under `~/.hugr/<name>/`.

Prerequisite: [tutorial 01](01-first-agent-cli.md) for the ask/answer/trace vocabulary. For the design rationale behind runtime embedding and its distinction from `hugr build --surface python`, see [the language surfaces documentation](../agents.md#language-surfaces).

## Install the package

The `hugr-agents` Python package wraps a PyO3 native module built from `crates/hugr-python`. From the repo root:

```bash
cd bindings/python
python3 -m venv .venv && . .venv/bin/activate
pip install maturin
maturin develop --release
```

`maturin develop` compiles the native extension in place and installs it into your venv. The import name is `hugr_agents`:

```python
import hugr_agents as hugr
```

The native crate (`hugr_agents._native`) embeds a tokio runtime and drives the real `hugr-agent` assembly path. A Python-defined agent therefore behaves like a manifest-defined one.

The boundary between the two layers is JSON strings. The native module owns the runtime and all validation. The pure-Python layer declares inputs with `TypedDict`s and recursively casts structured outputs into dataclasses.

## Define a tool

A tool is a callable plus an explicit JSON Schema; the advertised surface stays auditable, and Hugr never infers a schema from your signature. Wrap any callable with `@hugr.tool`:

```python
import hugr_agents as hugr

@hugr.tool(
    name="lookup_policy",
    description="Search policy text by keyword.",
    schema={
        "type": "object",
        "properties": {"query": {"type": "string"}},
        "required": ["query"],
    },
)
def lookup_policy(args):
    return {"matches": search_policy_text(args["query"])}
```

The decorator signature is `tool(fn=None, *, name=None, description="", schema=None, requires_permission=False, background=False)`. It works bare (`@hugr.tool`), called (`hugr.tool(fn, ...)`), or as a decorator factory (`@hugr.tool(...)`).

When `name` is omitted, it defaults to `fn.__name__`. When `description` is omitted, it falls back to the function's docstring. When `schema` is omitted, the tool gets `{"type": "object"}` with no required fields and accepts any args dict.

The callable takes one argument: a `dict` of decoded JSON args that matches the schema. It returns a JSON-serializable result.

### Async tools

The callable may be `async`. The runtime awaits it inside the tokio worker pool:

```python
@hugr.tool(name="lookup", description="d", schema={"type": "object"})
async def lookup(args):
    await some_async_work()
    return {"definition": "async ok"}
```

Sync and async tools are interchangeable from the agent's perspective, so choose the form that fits your I/O. A tool that raises an exception does not crash the run. The exception message is sent back to the model as a tool error result, allowing the model to recover and try again or finish the answer.

### The `requires_permission` and `background` flags

`requires_permission=True` marks a tool as gated. The model can call it, but the host's permission policy must approve it before execution. `background=True` marks a tool as fire-and-forget, so the result is not fed back to the model. Both are advanced flags for specific trust models; leave them at their defaults (`False`) for this tutorial.

## Assemble the agent

`hugr.Agent` is keyword-only, so every argument is named. The constructor is:

```python
agent = hugr.Agent(
    name="policy-helper",
    system="Answer from the policy tools. Return JSON.",
    models={
        "default": "medium",
        "base_url": "https://router.huggingface.co/v1",
        "api_key_env": "POLICY_API_KEY",
        "medium": {
            "model": "moonshotai/Kimi-K2-Instruct",
            "temperature": 0.2,
            "input_usd_per_m_tokens": 1.0,
            "output_usd_per_m_tokens": 1.5,
        },
    },
    tools=[lookup_policy],
    limits={"max_model_calls": 10, "timeout_s": 60},
)
```

The full signature is `Agent(*, name, system=None, models=None, tools=(), grants=None, limits=None, context=None, response_schema=None, version="0.0.0", description="", traces=None, scratchpad=None)`.

Each config key mirrors the corresponding `hugr.toml` section with the same names and shapes. The manifest details from [tutorial 01](01-first-agent-cli.md) therefore transfer directly.

The package exports `TierConfig`, `LimitsConfig`, `ContextConfig`, `GrantsConfig`, and the individual grant shapes as `TypedDict`s for static checking. `ModelsConfig` and the nested `mcp`/`agent` instance tables are typed mappings because tier selectors and external grant instance names are deliberately open strings.

### `models`

The `models` dict has three reserved keys (`base_url`, `api_key_env`, and `default`) plus one nested table per tier. A tier table requires a `model` id and optionally carries `temperature`, `max_tokens`, and per-million-token pricing (`input_usd_per_m_tokens`, `output_usd_per_m_tokens`). The `default` knob names which tier the agent uses. This is the exact shape of the `[models]` manifest block.

### `limits`

The `limits` dict accepts `max_model_calls`, `max_cost_micro_usd`, and `timeout_s`, matching the manifest's `[limits]` keys. Each key is optional; an unset key is unbounded.

### `grants`

`grants` is the Python name for the manifest's `[tools]` block, including library tools and the `mcp` and `agent` namespaces. Library tools are keyed by tool id (`{"fs_read": {...}, "web_fetch": {"allow_hosts": [...]}}`). MCP and agent grants nest one level deeper:

```python
agent = hugr.Agent(
    name="orchestrator",
    system="...",
    models={...},
    grants={
        "fs_read": {"root": "./docs"},
        "mcp": {"editor": {"command": "npx", "args": ["some-mcp-server"]}},
    },
)
```

Tools you define in Python (the `tools=[...]` list) are registered as capabilities alongside the granted library and external tools; they all show up in the agent card together.

### `context` and `response_schema`

`context` mirrors the manifest's `[context]` block for context projection and deterministic compaction.

`response_schema` is an optional JSON Schema dict. When set, the schema rides the provider request as `response_format`, and the final JSON is validated against it.

This is the pure-Python equivalent of the Rust `RESPONSE_RUST_TYPE` contract from [tutorial 02](02-typed-responses-and-hooks.md). Without a Rust type, validation occurs at the schema level rather than through a `serde` cast.

### `traces` and `scratchpad`

`traces` and `scratchpad` are optional string paths that override where traces and the scratchpad live, equivalent to the manifest's `[traces] store` and `[scratchpad] root`. When omitted, defaults apply (see "Where traces land" below).

## Ask a question

`agent.ask(question)` blocks until the turn finishes and returns an `Answer`:

```python
answer = agent.ask("Can I expense a train ticket?")
print(answer.status, answer.response, answer.trace_id)
```

The full signature is `ask(question, *, trace_id=None, blobs=(), extra=None)`.

`trace_id` resumes a prior conversation. The parent is re-folded into a fresh brain, and a new trace is written with `depends_on` set. Resuming the same parent twice creates a fork.

`blobs` is a sequence of `BlobHandle` objects (see below). `extra` is an opaque JSON-serializable value stamped into the trace header.

### The `Answer` type

`Answer` is a dataclass with `status`, `response`, `trace_id`, `metadata`, `blobs`, and `extra`. `status` is `hugr.STATUS_SUCCESS` or `hugr.STATUS_ERROR`. `response` is the user-facing payload dict, `metadata` is an `AnswerMeta`, and `blobs` is a list of `BlobHandle` values.

The `.ok` property is shorthand for `status == STATUS_SUCCESS`. Errors are answers, not exceptions. A blown limit, missing key, or model error returns `status == "error"` with `response == {"error": ...}`. The `trace_id` remains set so you can inspect what happened.

### `AnswerMeta`

`AnswerMeta` carries the mandatory cost accounting: `duration_ms`, `cost_micro_usd`, `tokens_in`, `tokens_out`, `model_calls`, `tool_calls`. Every field is an int defaulting to zero. These numbers come from the runtime's per-op fold; the same ones `hugr stats` aggregates.

### Passing blobs

`BlobHandle` is a dataclass with `ref` (a `BytesBlobRef`, `PathBlobRef`, or `Sha256BlobRef` dataclass), `media_type` (a string), and optional `name`. `from_bytes`, `from_path`, and `from_sha256` cover the three wire variants:

```python
blob = hugr.BlobHandle.from_path("./report.pdf", media_type="application/pdf")
answer = agent.ask("Summarize this report.", blobs=[blob])
```

`from_path(path, media_type="application/octet-stream", name=None)` builds a `PathBlobRef` for a local file the host reads. `from_sha256(sha256, media_type=..., name=None)` builds a content-addressed `Sha256BlobRef` into the shared blob store, while `from_bytes(base64, ...)` builds an inline `BytesBlobRef`. The file is materialized into the agent's scratchpad before the turn starts.

## Stream events

For live UIs or progress reporting, use `agent.run(...)` as an async iterator. It takes the same arguments as `ask` and yields the `AgentEvent` union; every variant and every structured nested value is a dataclass:

```python
import asyncio

async def stream():
    async for event in agent.run("Can I expense a train ticket?"):
        if isinstance(event, hugr.TextDeltaEvent):
            print(event.text, end="", flush=True)
        elif isinstance(event, hugr.ToolStartedEvent):
            print(f"\n[tool: {event.name}]")
        elif isinstance(event, hugr.AnswerReadyEvent):
            print(f"\n→ {event.answer.status}, trace {event.answer.trace_id}")

asyncio.run(stream())
```

Every event dataclass retains its literal `type` attribute for discriminated-union narrowing, while `isinstance` gives the most direct Python branch. The vocabulary is:

- `ask_started`: the turn began; carries `trace_parent` (the resumed parent's id, or `None`).
- `model_started`: a model call started; carries `op` and `tier` (the selector string).
- `text_delta`: a chunk of streamed assistant text; carries `op` and `text`.
- `model_ended`: a `ModelEndedEvent`; carries `op` and a `Usage` dataclass.
- `tool_started`: a tool call fired; carries `op`, `name`, and `args` (the decoded JSON).
- `tool_ended`: a tool call returned; carries `op`, `name`, `is_error` (bool), and `result`.
- `notice`: a free-form status message; carries `message`.
- `done`: a `DoneEvent`; carries a normalized `DoneReason` dataclass (`kind` is `end_turn`, `cancelled`, or `error`, with an optional error `message`).
- `answer_ready`: an `AnswerReadyEvent`; carries the full `Answer` dataclass.

The stream is guaranteed to start with `AskStartedEvent` and end with `AnswerReadyEvent`; the final answer is already available as `event.answer`.

## File feedback

Feedback is the asynchronous back-channel for recording, beside an immutable trace, whether an answer helped. It is never read during a live ask and is intended for offline analysis (see [tutorial 08](08-traces-replay-debugging.md)).

```python
answer = agent.ask("Can I expense a train ticket?")
fb = agent.feedback(answer.trace_id, {"score": 5, "note": "correct policy cited"})
assert fb.trace_id == answer.trace_id
```

`feedback(trace_id, payload)` returns a `Feedback` dataclass (`trace_id`, `payload`, `created_at_ms`). The payload is opaque JSON; Hugr never interprets it. Read it back with `feedback_for(trace_id)` which returns a `List[Feedback]`. Filing feedback on a nonexistent trace raises `RuntimeError`.

## Inspect and aggregate

Two methods give you the same audit views as the CLI flags from [tutorial 01](01-first-agent-cli.md):

```python
card = agent.describe()
print([tool.name for tool in card.tools])
print(card.model_tiers[0].selector, card.limits)

heads = agent.traces()
for h in heads:
    print(h.trace_id, h.status, h.question)

stats = agent.stats()
print(stats.totals.cost_micro_usd)
```

`describe()` returns an `AgentCard` dataclass with nested `ToolCard`, `ToolSchema`, `ModelTierCard`, `TierPrice`, and `AgentLimits` values. `traces()` returns a list of `TraceHead` dataclasses.

`stats(*, since=None, trace=None)` returns an `AgentStats` graph with typed totals, duration, per-trace, model, tool, and child-agent rows. Pass `since` to aggregate from a trace onward, or `trace` for one trace only.

If the assembly produced any warnings (e.g., a grant referencing an unknown library tool), they're available on the `agent.warnings` property as a list of strings.

## Where traces land

Traces persist under `~/.hugr/<agent-name>/traces/`, the same per-agent home used by every other surface. The agent name in the constructor names the directory.

Override the shared root with `HUGR_HOME`, or set one agent's home directly with `HUGR_AGENT_HOME`. The scratchpad lives at `~/.hugr/<name>/scratch/`. The shared blob store lives at `~/.hugr/blobs` and can be overridden with `HUGR_BLOB_STORE`.

The pytest suite in `bindings/python/tests/` sets `HUGR_HOME` to a temporary directory for each test.

## Verify with the Rust CLI

A trace written by a Python agent is a plain JSON file in the standard Hugr format. It contains no Python metadata and does not need Python to be read. The Rust CLI verifies it bit-for-bit:

```bash
hugr verify ~/.hugr/policy-helper <trace_id>
hugr replay ~/.hugr/policy-helper <trace_id> --step
```

This works because capability results (your Python tools' return values) are recorded as events in the trace; the replayed brain re-folds them without calling Python. The brain is sans-IO and pure, so its output is a pure function of the recorded input log. (See [tutorial 08](08-traces-replay-debugging.md) for the full replay/verify workflow.)

## A practical data-analysis agent

Python runtime embedding is especially useful when the capabilities the agent needs already live in a Python SDK or data stack. This example gives a subscription-retention agent controlled access to a pandas `DataFrame`: pandas does the deterministic filtering and arithmetic, while the model explains the result and recommends follow-up actions. The same pattern works with a warehouse client, analytics SDK, notebook library, or an internal Python package without putting those implementation details into Hugr core.

Pydantic dataclasses are a convenient source of JSON Schema for this boundary. `TypeAdapter.json_schema()` produces the explicit schema that Hugr advertises to the model, and `TypeAdapter.validate_python()` validates the decoded tool arguments before the callable uses them. Hugr does not depend on Pydantic or infer schemas from Python annotations; this is application code choosing Pydantic as its schema generator.

Install the two application dependencies next to `hugr-agents`:

```bash
pip install pandas "pydantic>=2"
```

Save the following as `run.py`. Set `RETENTION_API_KEY` to a key for the configured OpenAI-compatible endpoint before running it:

```python
from typing import Literal

import hugr_agents as hugr
import pandas as pd
from pydantic import Field, TypeAdapter
from pydantic.dataclasses import dataclass

ACCOUNTS = pd.DataFrame.from_records(
    [
        {"account_id": "acme", "segment": "growth", "monthly_revenue_usd": 2400.0, "failed_payments": 2, "weekly_logins": 1},
        {"account_id": "beacon", "segment": "startup", "monthly_revenue_usd": 450.0, "failed_payments": 0, "weekly_logins": 7},
        {"account_id": "cygnus", "segment": "enterprise", "monthly_revenue_usd": 9100.0, "failed_payments": 1, "weekly_logins": 2},
        {"account_id": "delta", "segment": "growth", "monthly_revenue_usd": 1800.0, "failed_payments": 3, "weekly_logins": 0},
    ]
)


@dataclass
class RiskQuery:
    segment: Literal["all", "startup", "growth", "enterprise"] = "all"
    min_failed_payments: int = Field(default=1, ge=0)
    max_weekly_logins: int = Field(default=2, ge=0)


@dataclass
class RiskAccount:
    account_id: str
    reason: str
    monthly_revenue_usd: float


@dataclass
class RetentionReport:
    summary: str
    accounts: list[RiskAccount]
    monthly_revenue_at_risk_usd: float
    recommended_actions: list[str]


risk_query = TypeAdapter(RiskQuery)
retention_report = TypeAdapter(RetentionReport)


@hugr.tool(
    name="find_at_risk_accounts",
    description="Find subscription accounts with both payment failures and low product usage.",
    schema=risk_query.json_schema(),
)
def find_at_risk_accounts(args):
    query = risk_query.validate_python(args)
    matches = ACCOUNTS[
        (ACCOUNTS["failed_payments"] >= query.min_failed_payments)
        & (ACCOUNTS["weekly_logins"] <= query.max_weekly_logins)
    ]
    if query.segment != "all":
        matches = matches[matches["segment"] == query.segment]

    accounts = [
        {
            "account_id": str(row.account_id),
            "segment": str(row.segment),
            "monthly_revenue_usd": float(row.monthly_revenue_usd),
            "failed_payments": int(row.failed_payments),
            "weekly_logins": int(row.weekly_logins),
        }
        for row in matches.itertuples(index=False)
    ]
    return {
        "accounts": accounts,
        "monthly_revenue_at_risk_usd": sum(
            account["monthly_revenue_usd"] for account in accounts
        ),
    }

agent = hugr.Agent(
    name="retention-analyst",
    system="""You investigate subscription churn risk.
Always call find_at_risk_accounts before answering.
Base every account and revenue figure on the tool result.
Return a RetentionReport JSON object and no additional fields.
""",
    models={
        "default": "medium",
        "base_url": "https://router.huggingface.co/v1",
        "api_key_env": "RETENTION_API_KEY",
        "medium": {
            "model": "moonshotai/Kimi-K2-Instruct",
            "input_usd_per_m_tokens": 1.0,
            "output_usd_per_m_tokens": 1.5,
        },
    },
    tools=[find_at_risk_accounts],
    limits={"max_model_calls": 4, "max_cost_micro_usd": 20_000, "timeout_s": 60},
    response_schema=retention_report.json_schema(),
)

answer = agent.ask(
    "Find growth accounts at risk using the default thresholds. "
    "Explain why each account qualifies and recommend the next action."
)
if not answer.ok:
    raise RuntimeError(answer.response["error"])

# Hugr validates the final JSON against the Pydantic-generated schema. Parse it
# into the same application type for typed downstream use.
report = retention_report.validate_python(answer.response)
print(report.summary)
for account in report.accounts:
    print(account.account_id, account.monthly_revenue_usd, account.reason)
print("MRR at risk:", report.monthly_revenue_at_risk_usd)
print("trace:", answer.trace_id)

# Inspect what landed on disk.
for head in agent.traces():
    print(head.trace_id, head.depends_on, head.status)
```

Run it with `python run.py`. The first run writes a trace to `~/.hugr/retention-analyst/traces/`. Verify it without Python:

```bash
hugr verify ~/.hugr/retention-analyst <trace_id_from_stdout>
```

There are two distinct validation points. Pydantic validates each tool call inside `find_at_risk_accounts`; an invalid threshold raises an exception, which Hugr returns to the model as a semantic tool error it can correct. Hugr validates the model's final object against `response_schema`; after a successful answer, `retention_report.validate_python(answer.response)` turns the opaque JSON payload into the application's typed dataclass graph.

### Resume and fork

Pass a prior answer's `trace_id` to continue the conversation. A new trace is written with `depends_on` pointing at the parent:

```python
follow_up = agent.ask("And what about flights?", trace_id=answer.trace_id)
assert follow_up.trace_id != answer.trace_id
heads = agent.traces()
by_id = {head.trace_id: head for head in heads}
assert by_id[follow_up.trace_id].depends_on == answer.trace_id
```

## A security note

Python callables are **trusted host code**. Hugr jails what the *model* can invoke (sandbox-by-registration; a tool the agent doesn't grant is a tool the model cannot call), not what your Python does once invoked. A tool that reaches outside its declared scope is a hole you drill, not one Hugr can close. (See the threat model in [the security documentation](../security.md).)

## Next

You've defined an agent entirely in Python. Next, see the same runtime from TypeScript through the `hugr-agents` package over the WASM brain in Node and the browser: [An agent entirely in TypeScript](06-agent-entirely-in-typescript.md).
