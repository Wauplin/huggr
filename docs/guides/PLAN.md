# Guide plan

This file tracks the guides still to write under `docs/guides/`. Guides 1 through 9 cover the surfaces (CLI, typed responses, browser, Python, TypeScript), composition and cost, traces and replay, and context compaction. The features below have reference documentation but no hands-on guide yet. Each entry becomes one guide in the style of [context compaction](09-context-compaction.md): problem first, mechanism, configuration, worked example, limitations. Remove an entry when its guide lands.

## Planned guides

All guides planned here have been written; see the [guides index](README.md). Add new entries as gaps appear.

## Covered elsewhere, no separate guide

- Typed response contracts and hooks: [guide 2](02-typed-responses-and-hooks.md).
- Agents as tools, delegation, feedback, `huggr stats`: [guide 7](07-composition-and-cost.md).
- Trace anatomy, replay, verify: [guide 8](08-traces-replay-debugging.md).
- Context compaction and pruning: [guide 9](09-context-compaction.md).
- The security model and per-capability threat notes stay in [the reference](../security.md); guide 10 links to them instead of restating.
- Custom storage backends and custom `TurnPolicy` implementations are advanced host extension points documented in [runtime](../runtime.md); a guide can follow if they stabilize.
