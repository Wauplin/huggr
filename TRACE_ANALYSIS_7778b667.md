# Trace analysis: huglet-docs `7778b667953149c3` (skill_read loop)

Analysis of trace `~/.huggr/huglet-docs/traces/7778b667953149c3.json`, produced by huglet-docs 0.0.6 built from commit `1ad0f94`. The ask was "how to upload files to a bucket?" and returned `status: success`, but only after cycling for most of the run. This document describes what happened, why, and what would have prevented it. No fixes are implemented here.

## Summary

The agent answered a one-file docs question in 44 model calls instead of the 3 to 5 it needed. After finding and reading the right document in the first two calls, the model spent 35 consecutive turns re-calling `skill_read({"name": "source-citation"})`, receiving the identical 147-token skill body every time, before finally emitting the answer on call 44. Nothing in the stack pushed back: the manifest declares no `[limits]`, the host re-executed and re-granted every identical call, and the full growing transcript was resent on each turn. The run consumed about 905K input tokens (roughly $0.91 at the configured `balanced` pricing) and 39 seconds for an answer whose useful work was done in the first 4 seconds.

This is not a one-off. Trace `49460767f2edfd3a` ("what is the hub", same agent version) shows the same shape with 70 model calls, including 60 consecutive identical `skill_read("source-citation")` calls, and about 1.94M input tokens.

## Timeline of the trace

| Op(s) | What happened |
|---|---|
| 0 | `fs_search("upload files to a bucket")` — immediately finds `docs/huggingface_hub/guides/buckets.md` |
| 2 | `fs_read` of that guide. The evidence for the answer is now fully in context |
| 4 | `skill_read("source-citation")` — legitimate first read of the only registered skill |
| 6, 10 | `skill_read("huglet_docs__DocsResponse")` — twice; both return `{"error": "unknown skill: huglet_docs__DocsResponse"}` |
| 8 | `fs_read` of the same buckets guide a second time |
| 12 | `skill_read("source-citation")` again |
| 14 | `scratch_write` of answer notes (the answer is essentially drafted here) |
| 16–84 | **35 consecutive `skill_read("source-citation")` calls**, each returning the identical skill body |
| 86 | Final text turn: a correct JSON `DocsModelResponse` citing `huggingface_hub/guides/buckets.md` |

Input tokens per call grew from 2,290 (op 0) to 25,176 (op 86); the total across 44 calls is 905,072 input tokens because the whole transcript, including every duplicated tool result, is resent on every turn. The loop's cost is quadratic in its length.

## Root cause

The direct cause is a repetition loop in the model. The `balanced` tier maps to `google/gemma-4-31B-it:cerebras` (from `~/.huggr/models.toml`, same values as the built-in catalog), a small model that is weak at long agentic tool-use sequences. Once it had drafted the answer (op 14), it entered a degenerate state where the most likely next action was the same `skill_read` call, and each identical result appended to the transcript reinforced the pattern instead of breaking it. It escaped on its own at op 86; the sibling trace shows the same escape after 60 iterations, so termination was luck, not design.

Several parts of the stack made the loop possible or made it expensive:

1. **No `[limits]` in the huglet-docs manifest.** The runtime supports `max_model_calls`, `max_cost_micro_usd`, and `timeout_ms` (`crates/huggr-agent/src/limits.rs`), and exceeding a limit produces a well-formed error answer with a persisted trace. huglet-docs sets none of them, so there was no backstop. A `max_model_calls` of even 20 would have capped both runs at a fraction of the cost.

2. **The host re-executes identical calls verbatim.** 37 successful `skill_read("source-citation")` calls each went through a fresh permission request (39 `RequestPermission` commands in the trace, all for `skill_read`) and each appended the full 147-token body to the transcript again. Nothing detects that a capability call is byte-identical to a previous one in the same ask, even for a read-only, deterministic tool.

3. **The skills prompt invites re-reading.** The generated system prompt says "When a skill matches the user's task, call `skill_read` before acting and follow the returned instructions." For a docs agent, the only skill matches every turn, and nothing says "once per ask". The skill body itself ("Search before answering, then read the smallest set of relevant files...") reads like a standing instruction, which a weak model can interpret as something to re-consult before acting each time.

4. **The response contract leaks a name that looks like a skill.** The typed contract is sent as `response_format.json_schema` named `huglet_docs__DocsResponse`. That name appears nowhere in the system prompt, the user block, or the tool list; it is only in the request `extra`. The model still called `skill_read("huglet_docs__DocsResponse")` twice (in both traces), so the provider evidently surfaces the schema, including its name, to the model server-side. From the model's point of view an undocumented name appeared in its context, and the prompt's own advice ("when a skill matches, call `skill_read`") sent it hunting. The resulting `unknown skill` error names no alternatives, so the model got no correction, just a dead end, right before the loop began.

5. **Nothing observes cost while a run is live.** The 39-second, 905K-token run looked identical to a healthy one until the trace was inspected after the fact. `huggr stats` can show the outlier afterwards, but there is no in-flight signal (log line, counter, or warning) when a single ask crosses an unusual number of model calls.

A note on the final answer quality: the response is correct and well-grounded, but it uses markdown headings and bold text despite the system prompt's "Do not use markdown formatting in your response", another symptom of the model tier being marginal for this agent's instructions.

## What would have prevented or contained this

In rough order of leverage; none of these are implemented in this change.

1. **Declare `[limits]` in every example manifest.** huglet-docs should ship with something like `max_model_calls = 20`, a `max_cost_micro_usd` budget, and a `timeout_ms`. The enforcement machinery already exists and produces a clean error answer; the manifests just do not use it. This is the cheapest containment and turns a $1-per-question failure mode into a bounded, visible error.

2. **Detect repeated identical capability calls in the host or policy.** When a turn issues a tool call byte-identical to one already answered in the same ask, the host (or a `TurnPolicy`) can short-circuit: return a compact marker such as `{"note": "identical call already answered at op 4"}` instead of the full body, or after N repeats end the turn with an error. This both breaks the reinforcement (the transcript stops accumulating identical rewarding results) and caps the token waste. It fits naturally at the policy layer, keeping the reducer strategy-free.

3. **Reconsider the model tier for agents with typed contracts.** `balanced` (gemma-4-31B) is demonstrably marginal for a multi-turn tool-use loop combined with a strict `response_format`: two of the four recorded traces degenerated. Either default docs-style agents to `powerful`, or have the policy escalate the selector when a turn count threshold is crossed.

4. **Tighten the skills prompt and the unknown-skill error.** Say explicitly that a skill needs to be read at most once per ask and that its instructions persist for the whole conversation. Make `skill_read`'s error for an unknown name list the available skills, so a bad guess gets corrected instead of dead-ending.

5. **Explain the response contract in the prompt instead of relying on provider-side schema injection.** The model should learn from the system prompt that its final message must be a JSON object with `response` and `related_documents`, and that this is a response format, not a tool or skill. That removes the incentive to probe `skill_read` with the schema name, and reduces dependence on how each provider chooses to surface `response_format` to the model.

6. **Surface in-flight anomaly signals.** A warning (frontend event or log line) when an ask exceeds, say, 15 model calls or a cost threshold would have made this visible on the first occurrence instead of after two multi-hundred-thousand-token runs.

## Supporting data

- Trace `7778b667953149c3`: 44 `ModelOutput` records, 43 `ToolResult` records, 905,072 input tokens, 1,201 output tokens, 39.2s wall clock, status `success`.
- Of the 43 tool results: 37 identical successful `skill_read("source-citation")` bodies, 2 `unknown skill: huglet_docs__DocsResponse` errors, 2 duplicate `fs_read` of the same guide.
- Trace `49460767f2edfd3a` (same agent version, earlier the same day): 70 model calls, 60 consecutive identical `skill_read("source-citation")` calls, ~1.94M input tokens, status `success`.
- For scale: the only other successful trace in the store, `897c6e9ef8273122`, completed in 1 model call and 2,043 input tokens.
