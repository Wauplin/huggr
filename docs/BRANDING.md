# Branding

## Name

**Hugr**

A Rust-based agent harness. Branded **Hugr**.

### Why "Hugr"

**Hugr** is an Old Norse word usually translated as "mind", "thought", "will", or "inner intent". That is the core of the project: a small, portable agent mind that can run inside many different bodies.

The name maps to the architecture cleanly:

- **Mind, not body** — the core is a pure sans-IO state machine. Hosts provide the body: files, shell, browser APIs, model transport, storage, UI.
- **One mind, many forms** — the same reducer can run in a native CLI, a browser extension, a Python/JS binding, or a server.
- **Thought over trace** — the durable event log is memory; projection is the current working thought; replay rebuilds the same mind from the same remembered events.
- **HF-adjacent without being cute** — it carries the "hug" sound and warmth, but avoids the toy-like feel of names such as "Huggy".

It is short, punchy, distinctive in writing, and credible next to projects named Pi, Codex, OpenClaw, and Hermes.

### Pronunciation

Default pronunciation: **HUG-er**.

The spelling is intentionally left as `Hugr`, not `Huger`, to keep the name compact, searchable, and mythic.

### Vocabulary

Do not force the whole codebase into a Norse metaphor. Keep the architecture vocabulary because it is clearer:

- **hugr** — the product / project name.
- **brain** / **core** — the pure sans-IO state machine.
- **host** — the environment-specific body that performs IO.
- **event** / **command** — the narrow waist between brain and host.
- **trace** — the durable event log and replay artifact.
- **capability** — a host-provided tool/effect.
- **policy** — externalized decisions for routing, permissions, and behavior.

The CLI reads naturally: `hugr run ...`.

### Taglines

- **The portable agent mind.**
- **One agent mind, any host.**
- **A tiny agent brain for every runtime.**
- **Replayable agents, anywhere.**

## Naming & namespaces

| Namespace           | Name   | Status                                           |
| ------------------- | ------ | ------------------------------------------------ |
| Brand               | `Hugr` | Chosen.                                          |
| crates.io (publish) | `hugr` | Appears available; reserve before launch.        |
| GitHub              | `hugr` | Bare user/org appears taken.                     |
| GitHub repo         | `hugr` | Use under an owning org, e.g. `huggingface/hugr`. |
| npm                 | `hugr` | Appears available; reserve before launch.        |

### Crate naming convention

Derivatives follow `hugr-<area>`:

- `hugr-core` — runtime.
- `hugr-cli` — the `hugr` command.
- `hugr-host` — batteries-included host utilities.
- `hugr-replay` — trace save/replay/inspect.
- `hugr-wasm` — WASM binding and browser host.
- `hugr-hub` — Hugging Face integration.

### Note on discoverability

`Hugr` has a better search profile than common English nouns, while still being short enough to say in conversation. The spelling will need one pronunciation hint early on, but the distinctiveness is a useful trade-off for owning the name over time.

Before public launch, do a final pass on package reservations, domain/social handles, and trademark risk. The table above is a working availability snapshot, not legal clearance.
