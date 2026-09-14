# Risks

Known hazards in the design, with the mitigation we intend. Severity is the risk *as currently
designed*, not as it would be with the mitigation applied.

---

### 1. Resident + reentrant conflict

**Severity: high.**

Fresh transient runs make re-entrancy safe — two concurrent runs of one agent share no state. But
a *pinned resident* agent has exactly one provider session, and two concurrent re-entrant runs
would resume and corrupt it.

*Mitigation:* resident agents serialize their runs; only transient agents are truly reentrant.
This makes residency a meaningfully different mode rather than an optimisation, which should be
documented as such.

### 2. Shared workspace contention

**Severity: high. Partially mitigated.**

Concurrent read-write agents share one working directory and *will* clobber each other's files.
There is no locking and no merge strategy.

*Partial mitigation:* agents now declare `access = "read-only" | "read-write"`, and read-only
agents receive a git worktree snapshot rather than the live tree — see
[`routing.md`](routing.md#5-workspace-access). This fully covers the common fan-out shape where
several agents inspect concurrently and at most one writes.

*Still unresolved:* **two concurrent read-write agents remain unsafe.** The route map does not
prevent that shape, and nothing warns you when you configure one. A load-time check that flags
fan-out to multiple read-write agents would be cheap and worth having.

### 3. Blocking chains hold processes open

**Severity: medium. Resolved by design.**

A `request_response` chain of depth N would keep N CLI processes alive and idle, each holding
memory and possibly a provider session. Hops was the only bound on depth.

*Resolved:* rendezvous joins provide fan-in without blocking anyone, so `request_response` is not
needed for v0.1 and is deferred indefinitely. The Tower parks flights instead of parking
processes. This also removed the pressure behind risk 1.

### 4. Fuel accounting depends on the CLI reporting cost

**Severity: critical.**

Claude Code reports token usage in `stream-json`. Codex and Copilot CLI report less consistently.
If a runner reports nothing, Fuel silently stops metering and the budget rail becomes fiction.

This was merely awkward while Fuel was a nice-to-have. Now that Fuel is the **only** bound on
breadth — Hops bounds depth alone — a silent metering failure removes the sole protection against
an exponential fan-out spending unbounded money.

*Mitigation, mandatory:* Fuel must degrade to a deterministic fallback that needs no runner
cooperation — a per-itinerary run cap, optionally wall-clock — whenever cost reporting is absent
or partial, and the Tower must log loudly when it does. A safety rail that fails silently is worse
than no rail, because it is trusted.

### 5. Agents rewriting their own static context

**Severity: medium.**

Agents given full write access to a workspace can edit that repository's `AGENTS.md` — the very
file shaping their behaviour. That is a drift loop with no human in it.

*Mitigation:* the Tower should treat instruction files as protected paths by default, overridable
in config. Note this applies to *users'* repositories; it does not apply here, because no factory
targets this repo.

### 6. Transcripts persisted but not exposed

**Severity: low.**

v0.1 can stream a live run but has no endpoint to read a finished one, despite storing full
transcripts and run metadata. Post-incident review would mean reading files by hand.

*Mitigation:* add `GET /runs/:id` and `GET /runs/:id/transcript` when the UI needs them.

### 7. Self-modification

**Severity: low — largely resolved.**

If a factory's workspace were Layover's own source, agents could edit the supervisor running them,
and a bad rebuild would brick the factory.

*Resolved by scope:* no factory ever targets this repository. The residual rule is that the Tower
always runs from an installed binary outside any workspace, never `cargo run` from inside one.

### 8. `layover.toml` as a single file

**Severity: low.**

Fine at ten agents, awkward at a hundred, and a merge-conflict magnet when several agents edit
the factory definition.

*Mitigation:* add `include = [...]` when it actually hurts. Not before.

### 9. Architecture drift between docs and code

**Severity: low, but grows.**

`docs/architecture.md` is normative and hand-maintained. Once code exists, nothing forces them to
agree, and an agent trusting a stale document will make confident wrong changes.

*Mitigation:* keep the decision log in `architecture.md` updated in the same commit as the change
it describes — this is why Conventional Commits and a `docs:` type matter.

### 10. Barrier leak

**Severity: high.**

A rendezvous join parks flights until all upstreams arrive. A failure loop-back diverts one branch
away from the join, so the barrier can never complete and the itinerary hangs holding parked work
forever. Hops does not help: nothing is flying.

*Mitigation:* the Tower abandons a barrier by **reachability** — if no live run in the itinerary
could still reach it, the barrier is dead and the itinerary is marked *stalled*. `timeout_sec` is
only a backstop. Stalled must be a distinct, visible outcome; silently parked work is worse than
a crash.

### 11. Barrier staleness

**Severity: high.**

After an upstream re-runs, a barrier may still hold a result produced by a *sibling* branch before
the re-run. The joined agent then acts on a mixture of old and new state — a correctness failure
that looks exactly like success.

*Mitigation:* a barrier resets when any upstream delivers a second time. This imposes a
constraint: a loop-back must re-dispatch the **whole** fan-out, not just the branch that failed.
The constraint currently lives in agent prompts, which makes it fragile — a load-time or runtime
check would be better and is not yet designed.
