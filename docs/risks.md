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
prevent that shape, and the load-time check only looks *within* one route's fan-out. A sequential
hand-off between two writers — a developer sending finished work to a publisher that commits it —
is not flagged at all, even though the sender's process may still be alive when the receiver
starts. Nothing today guarantees the sender has exited.

### 3. Blocking chains hold processes open

**Severity: medium. Resolved by design.**

A `request_response` chain of depth N would keep N CLI processes alive and idle, each holding
memory and possibly a provider session. Hops was the only bound on depth.

*Resolved:* rendezvous joins provide fan-in without blocking anyone, so `request_response` is not
needed for v0.1 and is deferred indefinitely. The Tower parks flights instead of parking
processes. This also removed the pressure behind risk 1.

### 4. Fuel accounting depends on the CLI reporting cost

**Severity: critical. Partially mitigated.**

Claude Code reports token usage in `stream-json`. Codex and Copilot CLI report less consistently.
If a runner reports nothing, Fuel silently stops metering and the budget rail becomes fiction.

This was merely awkward while Fuel was a nice-to-have. Now that Fuel is the **only** bound on
breadth — Hops bounds depth alone — a silent metering failure removes the sole protection against
an exponential fan-out spending unbounded money.

*Mitigation, in place:* the gap is measured rather than merely flagged. Every run records a
[`CostSource`] of `reported`, `rate_card` or `unreported`; an itinerary counts how many of its runs
went unmetered and exposes `metered_share()`; and every total in the ledger reports the *weakest*
source that fed it, so a mostly-measured figure still reads as an estimate. A rate card can price
a run that reported tokens but no dollars — labelled as an estimate, never folded in as a
measurement.

*Still unresolved:* none of this makes an unreporting runner report. The deterministic run cap is
what actually holds, and an operator has to look at `measured_share` to know whether Fuel is
metering or merely appearing to.

### 5. Spend that no per-chain budget can see

**Severity: high. Mitigated.**

Fuel is per itinerary. A scheduled pipeline mints a fresh itinerary — and a fresh Fuel budget — on
every tick, so an hourly pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day while every
chain stays perfectly inside its rail.

*Mitigation:* the **Reserve**, a rolling-window ceiling across every itinerary, checked before an
itinerary is minted. It rolls rather than resetting daily, because a calendar bucket can be spent
twice across midnight and needs a timezone to decide when midnight is. `layover validate` warns
when a factory has a scheduled pipeline and no Reserve, or a Reserve too small to fund one run of
it.

*Residual:* the Reserve is only as good as the cost figures feeding it, so risk 4 applies here
too — an unreporting runner spends against a Reserve that never decrements.

### 6. Agents rewriting their own static context

**Severity: medium.**

Agents given full write access to a workspace can edit that repository's `AGENTS.md` — the very
file shaping their behaviour. That is a drift loop with no human in it.

*Mitigation:* the Tower should treat instruction files as protected paths by default, overridable
in config. Note this applies to *users'* repositories; it does not apply here, because no factory
targets this repo.

### 7. Transcripts persisted but not exposed

**Severity: low.**

v0.1 can stream a live run but has no endpoint to read a finished one, despite storing full
transcripts and run metadata. Post-incident review would mean reading files by hand.

*Mitigation:* add `GET /runs/:id` and `GET /runs/:id/transcript` when the UI needs them.

### 8. Self-modification

**Severity: low — largely resolved.**

If a factory's workspace were Layover's own source, agents could edit the supervisor running them,
and a bad rebuild would brick the factory.

*Resolved by scope:* no factory ever targets this repository. The residual rule is that the Tower
always runs from an installed binary outside any workspace, never `cargo run` from inside one.

### 9. `layover.toml` as a single file

**Severity: low.**

Fine at ten agents, awkward at a hundred, and a merge-conflict magnet when several agents edit
the factory definition.

*Mitigation:* add `include = [...]` when it actually hurts. Not before.

### 10. Architecture drift between docs and code

**Severity: low, but grows.**

`docs/architecture.md` is normative and hand-maintained. Once code exists, nothing forces them to
agree, and an agent trusting a stale document will make confident wrong changes.

*Mitigation:* keep the decision log in `architecture.md` updated in the same commit as the change
it describes — this is why Conventional Commits and a `docs:` type matter.

### 11. Barrier leak

**Severity: high.**

A rendezvous join parks flights until all upstreams arrive. A failure loop-back diverts one branch
away from the join, so the barrier can never complete and the itinerary hangs holding parked work
forever. Hops does not help: nothing is flying.

*Mitigation:* the Tower abandons a barrier by **reachability** — if no live run in the itinerary
could still reach it, the barrier is dead and the itinerary is marked *stalled*. `timeout_sec` is
only a backstop. Stalled must be a distinct, visible outcome; silently parked work is worse than
a crash.

### 12. Barrier staleness

**Severity: high.**

After an upstream re-runs, a barrier may still hold a result produced by a *sibling* branch before
the re-run. The joined agent then acts on a mixture of old and new state — a correctness failure
that looks exactly like success.

*Mitigation:* a barrier resets when any upstream delivers a second time. This imposes a
constraint: a loop-back must re-dispatch the **whole** fan-out, not just the branch that failed.
The constraint currently lives in agent prompts, which makes it fragile — a load-time or runtime
check would be better and is not yet designed. The reference factory in
[`examples/workitem-factory/`](../examples/workitem-factory/README.md) turns this loop on every
rework round, and `a_rework_round_that_re_dispatches_only_one_branch_waits_forever` shows exactly
what a half re-dispatch costs.

### 13. Prompt composition has no output-size bound

**Severity: low.**

`@include` is guarded against cycles and against nesting more than eight deep, but not against
*breadth*. A non-cyclic diamond — eight levels where each file includes the same two children —
expands super-linearly, because cycle detection tracks the ancestor stack rather than a visited
set, and nothing caps the size of the assembled prompt.

The blast radius is small: prompt files are repository content under the same review as the rest
of the factory, and the failure is a large string rather than anything escaping the process. But
an agent editing its own prompts is a stated goal, which puts a machine on the writing end.

*Mitigation:* cap the assembled prompt at some generous size and fail with a clear error, the same
way depth does. Worth doing when prompts become agent-writable, not before.

### 14. Release pipeline trust

**Severity: low. Mostly mitigated.**

The release workflow holds `contents: write` and the Pages workflow holds `id-token: write`. Any
action running in those jobs runs with those tokens, so a compromised upstream would be a
release-pipeline compromise.

*Mitigation:* `.github/workflows/release.yml` is **generated by [`dist`](https://opensource.axo.dev/cargo-dist/)**
and uses only first-party `actions/*` — the three third-party actions the hand-rolled workflow
needed are gone, which was the larger exposure. In `ci.yml` and `pages.yml`, which are still
hand-written, third-party actions are pinned to full commit SHAs.

*Residual, accepted:* the generated workflow interpolates `github.ref_name` into one `run:` block,
which is the injection shape the hand-rolled workflow was fixed for. Exploiting it needs the
ability to push a `v*` tag — the workflow never runs on `pull_request` — so it is a
privileged-user issue rather than an external one. It is **not** patched locally: the file is
generated, `dist plan` would report a hand-edit as drift, and diverging from upstream to fix a
low-severity issue costs more than it saves. Revisit if `dist` addresses it upstream.

*Residual:* `actions/*` are pinned to major tags, which is the usual trade-off.
