# Roadmap

## v0.1 — prove the factory

The goal of v0.1 is narrow: demonstrate that agents can trigger one another unattended, that
several results can rendezvous, and that the whole thing is bounded in both depth and cost.

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

**Bounds**

10. Hops decrement per flight; a chain terminates at zero instead of running forever.
11. **Fuel debits against the itinerary and halts it when exhausted.** Hops bounds depth only, so
    Fuel is what bounds total work — see the fan-out arithmetic below.
12. **Fuel degrades to a deterministic fallback** — a per-itinerary run cap — whenever a runner
    cannot report cost, and the Tower logs loudly when it does. Without this, Fuel is not a rail.

**Observability and control**

13. Hangars are written: transcript, run metadata, memory.
14. `GET /runs` and `GET /runs/:id/stream` return live SSE output.
15. A minimal UI renders the route map, a live run feed, and stalled itineraries.
16. `POST /ground-stop` halts a running factory and stays halted across a Tower restart.

Criteria 8, 9, 11 and 12 are the ones that matter. Everything else is plumbing.

### Why Fuel is not optional

Hops decrements **per flight**, and branches inherit the remaining count rather than splitting it.
Hops therefore bounds *depth*, not *breadth*:

```
max_hops = 8, branching factor 3   →   up to 3^8 ≈ 6,500 runs from one trigger
```

Every one of those is a real CLI invocation costing real money. Hops alone is **not** the
difference between a factory and a fork bomb — Fuel is. This is why Fuel moved into v0.1, and why
criterion 12 exists: a cost rail that depends on runners voluntarily reporting cost is not a rail
at all.

### Explicitly out of scope for v0.1

- Resident agents
- `request_response` mode — **no longer needed**; rendezvous joins cover fan-in without holding
  processes open, which retired two risks rather than deferring them
- Two concurrent read-write agents
- Anything multi-machine
- Pointing a factory at Layover's own source — permanently out of scope

---

## Open questions

Agents should ask rather than guess on any of these.

### Run bootstrap — blocks implementation

1. **What does a run actually receive?** How do the agent's configured `prompt`, the incoming
   flight `body`, and the agent's `memory.md` combine into a single CLI invocation? Nothing
   specifies the composition or its order.
2. **Is memory injected or fetched?** Either the Tower splices `memory.md` into the prompt, or the
   agent must call `layover_memory_read()` itself. Because runs are fresh, this one choice decides
   whether explicit memory actually works — an agent that forgets to look never remembers anything.
3. **How does `model` reach each CLI?** The runner command templates have no model placeholder,
   and each CLI spells the flag differently.
4. **Does an agent know its own name?** Needed for self-reference in prompts and for reasoning
   about its own position in the mesh.

### Lifecycle semantics

5. **What does Ground Stop actually do?** Kill running processes, or only block new spawns? What
   happens to parked barriers, and are outstanding MCP tokens revoked?
6. **What is a dead-end run?** An agent exits without sending anything. Success, or a silently
   abandoned branch? This directly affects barrier reachability analysis.
7. **What happens on timeout?** `timeout_sec` exists in config, but nothing defines the behaviour
   when it fires.
8. **What happens when a run crashes?** Distinct from an agent reporting failure: the process
   dies or exits non-zero. Retry, dead-letter, notify the sender, or stall the itinerary.
9. **When is an Itinerary done?** Exhausting Hops or Fuel is failure, not success. Without a
   terminal state the UI cannot distinguish finished from idle.

### Concurrency edges

10. **Two humans POST simultaneously** — one itinerary or two?
11. **An agent reachable by both a joined and an unjoined edge** — which behaviour applies?
12. **How many loop-backs before giving up?** Hops bounds it, but exhausting Hops mid-repair can
    leave half-finished work in the shared workspace and no one to clean it up.
13. **Can the barrier-reset constraint be enforced?** A loop-back must re-dispatch the whole
    fan-out, and today that lives only in agent prompts.

### Platform

14. **Windows support.** Development happens on Windows, where process-tree termination, git
    worktrees and signal handling all differ from Unix. Ground Stop and timeouts are the
    OS-specific parts.

### Everything else

15. **How do child CLIs receive credentials?** Inherited environment, or injected per run by the
    Tower. Per-run injection allows per-agent credentials and revocation.
16. **Are prompts inline in TOML?** Longer prompts read poorly in config.
    `prompt_file = "prompts/planner.md"` gives better diffs and lets agents edit them.
17. **Is the Logbook one flat file or namespaced?** One file serializes every write in the factory
    through a single lock.
18. **Observability** — structured logs, or OpenTelemetry traces where an Itinerary is a trace and
    each Run a span? The latter would visualise a stalled barrier.
19. **UI stack** — plain HTML with SSE, or a framework. Reversible; it only consumes the HTTP API.
20. **Is the aviation terminology confirmed?** Adopted provisionally throughout. Cheap to strip
    while there is no code.

### Resolved

- **Does an agent see who sent a flight?** *Yes — mandatory.* A joined agent receives several
  flights at once and must be able to tell them apart.
- **What does a fan-out cost in Hops?** One hop per flight; branches inherit the remaining count
  rather than splitting it. Hence the breadth problem above.

---

## Beyond v0.1

Rough ordering, not commitments.

- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- `join = "any"`, and quorum joins
- `include = [...]` for multi-file factory definitions
- Additional runners: Gemini CLI, Aider, arbitrary command templates
