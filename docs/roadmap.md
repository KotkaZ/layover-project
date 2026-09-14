# Roadmap

## v0.1 — prove the factory

The goal of v0.1 is narrow: demonstrate that agents can trigger one another unattended, that
several results can rendezvous, and that the whole thing is bounded.

### Done criteria

**Configuration**

1. `layover.toml` parses: agents with prompt, model, runner, `access` and `entry`; routes with
   direction, fan-out and `join`.
2. Validation at config load rejects unknown agent names and unreachable entry points, and warns
   on fan-out to multiple read-write agents.

**Execution**

3. `layover run` starts the Tower — HTTP server and MCP server together.
4. `POST /flights` triggers a real `claude -p` run inside the shared workspace.
5. That agent calls `layover_send` and successfully triggers a second, different agent.
6. An agent fans out to two peers, and they run **concurrently**.
7. A read-only agent receives a git worktree snapshot rather than the live tree.

**Rendezvous**

8. `join = "all"` parks flights and spawns the target **exactly once**, with all inputs.
9. A barrier that can no longer be reached is abandoned, and the itinerary is marked **stalled**
   and displayed as such — distinctly from failed.

**Safety and observability**

10. Hops decrement across the chain, and a chain terminates at zero instead of running forever.
11. Hangars are written: transcript, run metadata, memory.
12. `GET /runs` and `GET /runs/:id/stream` return live SSE output.
13. A minimal UI renders the route map, a live run feed, and stalled itineraries.
14. `POST /ground-stop` halts a running factory and stays halted across a Tower restart.

Criteria 8–10 are the ones that matter. Everything else is plumbing; the barrier is the difference
between a rendezvous and a race, and the Hops bound is the difference between a factory and a
fork bomb.

### Explicitly out of scope for v0.1

- Resident agents
- `request_response` mode — **no longer needed**; rendezvous joins cover fan-in without holding
  processes open, which retired two risks rather than deferring them
- Fuel accounting
- Two concurrent read-write agents
- Anything multi-machine
- Pointing a factory at Layover's own source — permanently out of scope

---

## Open questions

Agents should ask rather than guess on any of these.

1. **What happens when a run crashes?** Distinct from an agent reporting failure: the process
   dies, exits non-zero, or times out. Retry, dead-letter, notify the sender, or stall the
   itinerary. Unattended operation makes this far more important than it looks.
2. **How do child CLIs receive credentials?** Inherited environment, or injected per run by the
   Tower. Per-run injection is more work but allows per-agent credentials and revocation — not
   every agent needs a token that can push.
3. **Can the barrier-reset constraint be enforced?** A loop-back must re-dispatch the whole
   fan-out, and today that lives only in agent prompts. A load-time or runtime check would be
   sturdier, and is not yet designed.
4. **Are prompts inline in TOML?** Longer prompts carry behavioural constraints and read poorly
   in a config file. `prompt_file = "prompts/planner.md"` gives better diffs and lets agents edit
   them.
5. **Is the Logbook one flat file or namespaced?** One file serializes every write in the factory
   through a single lock.
6. **Observability** — structured logs, or OpenTelemetry traces where an Itinerary is a trace and
   each Run a span? The latter fits almost too neatly, and would visualise a stalled barrier.
7. **When is an Itinerary done?** Hops bound it, but exhausting a budget is failure, not success.
   Without a terminal state the UI cannot distinguish finished from idle.
8. **How many loop-backs before giving up?** Hops bounds it, but exhausting Hops mid-repair can
   leave half-finished work in the shared workspace and no one to clean it up.
9. **UI stack** — plain HTML with SSE, or a framework. Reversible; it only consumes the HTTP API.
10. **Is the aviation terminology confirmed?** Adopted provisionally throughout. Cheap to strip
    while there is no code.

### Resolved by the join design

- **Does an agent see who sent a flight?** *Yes — now mandatory.* A joined agent receives several
  flights at once and must be able to tell them apart. Sender identity became a requirement
  rather than a preference.

---

## Beyond v0.1

Rough ordering, not commitments.

- Fuel accounting, degrading to wall-clock or run count when a runner cannot report cost
- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- `join = "any"`, and quorum joins
- `include = [...]` for multi-file factory definitions
- Additional runners: Gemini CLI, Aider, arbitrary command templates
