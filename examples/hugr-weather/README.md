# hugr-weather

A tiny, self-contained Hugr weather agent. It uses only the allowlisted `web_fetch` tool (jailed to the Open-Meteo API hosts in `hugr.toml`), so there is nothing to set up beyond a provider key.

## Run it

```bash
export HUGR_API_KEY=...            # your model provider key
hugr run . "what's the weather in Paris?"
```

The answer is the standard Hugr `Answer` JSON; `response.response` is the one-sentence weather summary.

## Next steps

- Edit `SYSTEM.md` to change the assistant's behavior or output style.
- Edit `allow_hosts` in `hugr.toml` to point `web_fetch` at other APIs.
- Adjust the response contract in `src/lib.rs` (currently a single string).
- Build a standalone binary: `hugr build . --release`.
- Inspect runs: `hugr traces .`, then `hugr replay`/`hugr verify`.
