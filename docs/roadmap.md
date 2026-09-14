# Roadmap

## v0.1 — prove the factory

The goal of v0.1 is narrow: demonstrate that one agent can trigger another, unattended, with a
bound on how far that can go.

### Done criteria

1. `layover.toml` parses — agents with prompt, model and runner; routes with direction and mode.
2. Route validation rejects unknown agents and unreachable entry points **at config load**, not
   at first flight.
3. `layover run` starts the Tower: HTTP server and MCP server together.
4. `POST /flights` triggers a real `claude -p` run inside the shared workspace.
5. That agent calls `layover_send` and successfully triggers a second, different agent.
6. Hops decrement across the chain, and a chain terminates when Hops reach zero instead of
   running forever.
7. Hangars are written: transcript, run metadata, memory.
8. `GET /runs` and `GET /runs/:id/stream` return live SSE output.
9. A minimal UI renders the route map and a live run feed.
10. `POST /ground-stop` halts a running factory, and it stays halted across a Tower restart.

Criterion 6 is the one that matters most. Everything else is plumbing; the Hops bound is the
difference between a factory and a fork bomb.

### Explicitly out of scope for v0.1

- Resident agents
- `request_response` mode — see [`risks.md`](risks.md#3-blocking-chains-hold-processes-open)
- Fuel accounting
- Workspace contention mediation
- Anything multi-machine
- Pointing a factory at Layover's own source — permanently out of scope

### Recommendation under consideration

**Cut `request_response` from v0.1 entirely.** It causes risks 1 and 3, and fire-and-forget alone
is sufficient to prove the concept. Blocking calls can be added once the async path is solid.

---

## Open questions

Unresolved. Agents should ask rather than guess on any of these.

1. **Does an agent see who sent a flight?** Sender identity enables trust decisions and loop
   detection, but also lets agents form behaviour that depends on the topology rather than the
   task.
2. **What happens when a run fails?** Retry, dead-letter queue, notify the sender, or drop
   silently. Unattended operation makes this far more important than it looks.
3. **Are prompts really inline in TOML?** Long system prompts may want
   `prompt_file = "prompts/planner.md"` — better diffs, and editable by agents.
4. **Is the Logbook one flat file or namespaced sections?** One file serializes every write in the
   factory through a single lock.
5. **Observability** — structured logs only, or OpenTelemetry traces where an Itinerary is a
   trace and each Run a span? The latter fits the model almost too neatly.
6. **How does a factory ever stop?** With no human in the loop, what does "done" mean for an
   Itinerary? Hops bound it, but exhausting a budget is failure, not success.
7. **Secrets** — how do child CLIs receive API keys? Inherited environment, or injected per run by
   the Tower? The latter allows per-agent credentials and revocation.
8. **UI stack** — plain HTML with SSE, or a framework? It only consumes the HTTP API, so this is
   reversible.
9. **Is the aviation terminology confirmed?** It is adopted throughout this repo provisionally.
   Straightforward to strip back to generic terms while there is no code.

---

## Beyond v0.1

Rough ordering, not commitments.

- Fuel accounting with graceful degradation when a runner cannot report cost
- `writable_paths` per agent, to make shared-workspace factories safe
- Resident agents with serialized runs
- `request_response`, if it proves necessary
- `include = [...]` for multi-file factory definitions
- Additional runners: Gemini CLI, Aider, arbitrary command templates
