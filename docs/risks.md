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
needed and is deferred indefinitely. The Tower parks flights instead of parking
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

The design can stream a live run but has no endpoint to read a finished one, despite storing full
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

### 15. Budgets are metered after the fact, not reserved before it

**What could happen.** `Reserve::authorize` is given no proposed liability and tracks nothing
in flight. With a `$40` Reserve and four Slots, four runs each costing `$20` all authorise while
recorded spend is still zero, and the factory ends `$40` over. Fuel has the same shape: a run
admitted with one cent remaining may spend a hundred dollars before its completion report arrives.

**Why it is not fixed.** A reservation needs a *worst-case estimate* before a run starts, and the
only honest source for that is a rate card and a token ceiling the Tower controls — neither of
which can be wired up before the supervisor exists. Guessing a ceiling now would either throttle
real work or provide false assurance.

**Mitigation until then.** Slots bound how many runs can be simultaneously overshooting, so the
overshoot is bounded by `max_concurrent_runs × worst single run` rather than being open-ended.
Size the Reserve with that headroom in mind.

### 16. A rail bounds Layover's own runs, not what a child does inside one

**What could happen.** Hops, Fuel, Slots and the route map govern Tower-mediated flights and runs.
They say nothing about what a child process does once started: it runs as the same OS user, the
shipped runners pass `--allow-all-tools`, and MCP wiring grants a whole server rather than named
tools. A compromised reviewer can use a write-capable MCP tool to open a pull request directly,
bypassing the publisher route and its `recovery = "manual"` policy entirely — and consuming no
Hops, because none of it is a flight.

**Why it is not fixed.** Real containment means a separate OS identity or a sandbox, plus per-tool
MCP allowlists. Both belong with process supervision.

**What this means for reading the rails.** They bound *the shape of the factory*, not the
authority of an agent. An agent given a write-capable tool has that authority whatever the route
map says. Grant tools as narrowly as the runner allows.

### 17. External effects have no idempotency key

**What could happen.** A layover is picked up by reading a `Booked` record, starting an itinerary,
and later amending it to `Resumed`. A crash between the second and third steps leaves it `Booked`,
so the next sweep starts a second itinerary for the same work — and both are *initial* runs, so
`recovery = "manual"` never applies. Automatic recovery has the mirror problem: `ChildState::Gone`
proves the local process ended, never that the pull request it was opening did not succeed.

**Why it is not fixed.** Exactly-once external effects need a durable claim with a lease, and a
stable operation identity carried across recovery, layover pickup and spawn. That is supervisor
work, and doing half of it would be worse than none.

**Mitigation until then.** `recovery = "manual"` on the one irreversible agent in the reference
factory, and the handover text telling a resumed run to check whether the earlier one already did
the thing. Both rely on the agent looking, which is weaker than a key.

### 18. A runner that under-reports its cost defeats both money rails

**What could happen.** Fuel and the Reserve are debited from a figure the child CLI prints about
itself. Negative, `NaN` and infinite values are already ignored, and `$0` alongside real tokens is
treated as silence rather than as a measurement — but a runner that reports a plausible-looking
`$0.01` for a `$4` run is believed. Both money rails then under-count by the same factor, and the
only rail left is `max_runs`, which counts invocations rather than money.

Spawn compounds it: each spawned itinerary is minted with fresh Fuel and a fresh run cap, so
generation depth bounds how deep spawning goes and nothing bounds how wide. The Reserve is meant
to be the factory-wide backstop, and it reads the same untrusted number.

**Why it is not fixed.** Nothing reports a cost yet, because nothing spawns a process. Deciding
what to do about an implausible figure — clamp to a rate card, treat as unreported, refuse the
runner — is a Tower behaviour and depends on what the CLIs actually emit.

**Mitigation until then.** `max_runs` needs no cooperation from the child and is the honest rail.
`CostSource` records where every figure came from and the weakest source wins, so a total that is
partly unmeasured says so rather than looking precise.

### 19. A learning is a standing instruction that no human approved

**What could happen.** A learning is injected into the next twenty runs of an agent as soon as it
is proposed. The text comes from an agent whose input may include a work item, a pull request
comment or a sibling's flight body — none of which are trusted. An instruction that survives
twenty runs, and can be re-proposed toward permanence, is a more durable foothold than any single
prompt injection.

`Impact` is already not trusted — the agent proposes it and the ledger recomputes it — but the
text was too.

**What was done (0.19.0).** Text is screened before it is stored: anything overriding instructions,
naming a `layover_*` tool, carrying a URL, shaped like a credential, or containing section markers
is refused, with a reply saying what an acceptable learning looks like. In the prompt each learning
is quoted, flattened onto one line, and introduced by a paragraph saying it is remembered text
rather than instruction.

Screened on the way in rather than filtered on the way out: storing it and hiding it later leaves
the thing an attacker wanted in the factory's memory, waiting for the filter to be relaxed.

**What is still open.** This is a filter on obvious attempts. An attacker who phrases an
instruction as an observation gets through, and no wordlist fixes that. The remaining mitigations
are structural and unchanged: expiry bounds the blast radius to twenty runs, an echo cannot
confirm a learning, it is presented as a claim, and a person can drop one in a press.

An approval queue is still rejected: it puts a human in a loop the factory exists to keep them out
of, and a sibling project's queue held 88 learnings after 22 days with none ever approved.

### 20. The prompt sandbox is lexical, so a symlink leaves it

**What could happen.** `@include` confines paths by resolving `..` lexically and refusing absolute
paths, drive prefixes and UNC paths. It does not canonicalise, so a symlink *inside* the prompt
directory pointing at `~/.ssh/id_rsa` or a `.env` is followed, and its contents are composed into
the prompt handed to a child CLI.

**What was done (0.19.0).** Both root and target are canonicalised, and the resolved target must
remain under the resolved root. Both checks now run: the lexical one refuses the obvious form
without touching the filesystem, the canonicalising one catches the form that looks innocent. A
path that cannot be canonicalised — because the file does not exist — is left to the read, so a
missing prompt is still reported as missing rather than as an attack.

**What is still true.** This is not yet a meaningful boundary, for the reason above: anyone who can
plant a symlink can also set `runners.*.command`, which is arbitrary code by design. It lands now
because it becomes load-bearing the moment agents write their own prompts, and doing it then would
mean doing it under pressure.
