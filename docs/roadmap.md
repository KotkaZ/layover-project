# Roadmap

## Where things stand

**v0.2 is released.** It is everything that happens *before* the first process is spawned:

- `layover.toml` parses and validates, including agents with identity, pipelines with triggers and
  flags, and prompts composed from files.
- The HTTP surface is specified in `api/openapi.yaml` and the server is generated from it.
- `layover` installs from crates.io and offers `validate`, `explain` and `prompt`.
- Documentation is published to GitHub Pages from `book/`.

**v0.1 — the part that actually runs a factory — is still open.** Everything below is unbuilt.

## v0.1 — prove the factory

The goal of v0.1 is narrow: demonstrate that agents can trigger one another unattended, that
several results can rendezvous, and that the whole thing is bounded in both depth and cost.

### Done criteria

**Configuration** — *done in v0.2*

1. `layover.toml` parses: agents with prompt, model, runner, `access`, `description` and
   `purpose`; routes with direction, fan-out and `join`; pipelines with triggers and flags.
2. Validation at config load rejects unknown agent names and unreachable entry points, and warns
   on fan-out to multiple read-write agents, undescribed agents, agents beyond the hop budget, and
   schedules faster than their own runs.

**Execution**

3. `layover run` starts the Tower — HTTP server and MCP server together.
4. `POST /flights` triggers a real `claude -p` run inside the shared workspace.
5. That agent calls `layover_send` and successfully triggers a second, different agent.
6. An agent fans out to two peers, and they run **concurrently**.
7. A read-only agent receives a git worktree snapshot rather than the live tree.

**Triggers**

8. A manual pipeline starts on `POST /flights`, carrying the flags the request set.
9. A scheduled pipeline fires on its own, without a request, and does not overlap itself silently.

**Rendezvous**

10. `join = "all"` parks flights and spawns the target **exactly once**, with all inputs.
11. A barrier that can no longer be reached is abandoned, and the itinerary is marked **stalled**
    and displayed as such — distinctly from failed.

**Bounds**

12. Hops decrement per flight; a chain terminates at zero instead of running forever.
13. **Fuel debits against the itinerary and halts it when exhausted.** Hops bounds depth only, so
    Fuel is what bounds total work — see the fan-out arithmetic below.
14. **Fuel degrades to a deterministic fallback** — a per-itinerary run cap — whenever a runner
    cannot report cost, and the Tower logs loudly when it does. Without this, Fuel is not a rail.

**Observability and control**

15. Hangars are written: transcript, run metadata, memory.
16. `GET /runs` and `GET /runs/:id/stream` return live SSE output.
17. A minimal UI renders the route map, a live run feed, and stalled itineraries.
18. `POST /ground-stop` halts a running factory and stays halted across a Tower restart.

**The reference scenario**

19. [`examples/workitem-factory/`](../examples/workitem-factory/README.md) runs end to end: a
    request reaches the analyst from either pipeline, both helpers are consulted concurrently and
    rendezvous back onto the analyst, the developer builds, the tester and reviewer report
    together, the developer loops until both approve, and the publisher opens a pull request.

This is **the** v0.1 scenario. It is what the design is sized against, and it is the reason
criteria 10, 11, 13 and 14 exist. It exercises two things nothing else does: a rendezvous landing
on an agent that ordinary edges also reach, and a loop that turns an unknown number of times. Its
configuration is parsed, validated and exercised by
`crates/layover-core/tests/workitem_factory*.rs`.

Criteria 10, 11, 13 and 14 are the ones that matter. Everything else is plumbing.

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
11. **How many loop-backs before giving up?** Hops bounds it, but exhausting Hops mid-repair can
    leave half-finished work in the shared workspace and no one to clean it up.
12. **Can the barrier-reset constraint be enforced?** A loop-back must re-dispatch the whole
    fan-out, and today that lives only in agent prompts.
13. **Can a join wait only for the upstreams that were actually dispatched?** `join = "all"` waits
    for every *declared* upstream, so an agent that consults a specialist only when the work needs
    one strands its own rendezvous. The workaround is to dispatch everyone every time and let the
    idle specialist reply "nothing to add" — see
    [`routing.md`](routing.md#a-join-has-no-optional-upstreams). A real fix needs the Tower to
    observe dispatch, and it has a race: a fast upstream could release the barrier before its
    sibling has been dispatched at all.

### Surviving a restart

**Settled: never resume a process. Start a new run and hand it the old one's state.** Both
recovery and steering use the one mechanism — see
[the decision log](architecture.md#13-decision-log) and
`crates/layover-core/src/handover.rs`. The domain model and its rails are built; what remains needs the Tower:

14. **How does the Tower notice an interruption?** A run that was live when the Tower died leaves
    no exit code. Distinguishing "was running when we crashed" from "finished while we were away"
    means writing run state before spawning and reconciling on startup.
15. **What goes in the handover's `progress` list?** The model carries whatever the store can
    honestly supply. Whether that is the agent's `memory.md`, a summary of its transcript, or
    the last N lines is a question about the store, which does not exist yet.
16. **Does a parked barrier survive a restart?** Its upstreams' runs did not. Re-dispatching the
    whole fan-out is the rule for a loop-back; the same probably applies here, which would mean a
    recovered itinerary discards partial barrier state.
17. **Is `interrupted` a third outcome beside `failed` and `stalled`?** It has to be visible, for
    the same reason `stalled` does — but it may be a state a human resolves rather than an ending.
### Platform

18. **Windows support.** Development happens on Windows, where process-tree termination, git
    worktrees and signal handling all differ from Unix. Ground Stop and timeouts are the
    OS-specific parts.

### Everything else

19. **How do child CLIs receive credentials?** Inherited environment, or injected per run by the
    Tower. Per-run injection allows per-agent credentials and revocation.
20. **Is the Logbook one flat file or namespaced?** One file serializes every write in the factory
    through a single lock.
21. **Observability** — structured logs, or OpenTelemetry traces where an Itinerary is a trace and
    each Run a span? The latter would visualise a stalled barrier.
22. **UI stack** — plain HTML with SSE, or a framework. Reversible; it only consumes the HTTP API.
23. **Is the aviation terminology confirmed?** Adopted provisionally throughout. Now that code
    exists it is no longer free to strip, but it is still only names.
24. **Does a scheduled pipeline skip a tick it is still working on, or start a second run?**
    Runs are reentrant, so today it would start a second. Validation warns when the firing gap is
    shorter than `timeout_sec`, which is a smell test rather than an answer. An independent review
    argued the safe default for unattended spending is to *skip* the tick and require
    `overlap = "allow"` to opt in; that is probably right and is a Tower behaviour, so it is
    recorded here rather than guessed at.
25. **What time zone does a cron expression mean?** Local to the Tower is the obvious answer and
    the obvious source of a 1am surprise twice a year.
26. **Should `entry = true` survive at all?** A review argued it is two ways to do one thing, and
    that a pipeline with no flags expresses the same intent. The counter-argument is in the
    decision log. The deciding evidence would be whether anyone actually uses a bare entry agent
    once pipelines exist; nobody has used either yet.

### Resolved

- **Does an agent see who sent a flight?** *Yes — mandatory.* A joined agent receives several
  flights at once and must be able to tell them apart.
- **What does a fan-out cost in Hops?** One hop per flight; branches inherit the remaining count
  rather than splitting it. Hence the breadth problem above.
- **An agent reachable by both a joined and an unjoined edge — which behaviour applies?**
  *The barrier wins only for the upstreams it names.* A flight from any other permitted sender
  bypasses the barrier, wakes the agent on its own, and leaves parked state untouched. The
  alternative — gating every inbound edge — would park an entry agent's own trigger while it
  waited for helpers that cannot run until it dispatches them. Settled by the reference scenario,
  where both barriers land on an agent that ordinary edges also reach. See
  [`routing.md`](routing.md#what-a-barrier-does-and-does-not-guard).
- **Are prompts inline in TOML?** *Both.* `prompt` for short instructions, `prompt_file` for
  anything that needs composing. Exactly one of the two; setting both is an error rather than an
  undefined precedence. See [the prompt reference](../book/src/prompts.md).
- **Can one factory definition serve several situations?** *Yes, through pipeline flags and
  conditional `@include` directives* — booleans chosen at trigger time, not a templating language,
  so every possible prompt stays a file somebody can read.
- **How does an agent get MCP servers of its own?** *Declared per agent, in `[agents.<name>.mcp.<server>]`.* Credentials are named in `env_from` and read from the Tower's environment rather than written into a committed file, and validation refuses a literal that looks like one.
- **Can several instances of one pipeline run at once?** *Yes.* Each trigger already mints its own itinerary, barriers and flags; `workspace = "per-itinerary"` adds the missing piece by giving each a git worktree of its own.
- **What happens to work that was in flight when the Tower died?** *A new run is started and handed the old one's state — never a resumed process.* Recovery is bounded by `max_recovery_attempts` on top of Hops, Fuel and the run cap, and a Ground Stop is never restarted through.
- **Can a human steer a run that is already going?** *Yes, by starting a new run carrying their instruction and the prior run's state.* Same mechanism as recovery, which is why neither needed resident agents.

---

## Beyond v0.1

Rough ordering, not commitments.

- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- `join = "any"`, quorum joins, and joins that wait only for the upstreams actually dispatched
- Per-agent MCP servers, pushed on by the reference scenario's telemetry agent
- Non-boolean pipeline parameters, if booleans turn out not to be enough
- `include = [...]` for multi-file factory definitions
- Additional runners: Gemini CLI, Aider, arbitrary command templates
