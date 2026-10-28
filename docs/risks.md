# Risks

Known hazards in the design, with the mitigation we intend. Severity is the risk *as currently
designed*, not as it would be with the mitigation applied.

Risks about releases and updating — 14 and 26 to 29 — are in
[`risks-releases.md`](risks-releases.md). The numbers are shared, so a risk keeps its number
wherever it is written down.

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

Concurrent agents share one working directory and *will* clobber each other's files. There is no
locking and no merge strategy.

*Made real by parallel runs.* Until 1.4.0 the Tower happened to run one agent at a time, which hid
this for agents in different chains. Runs now overlap up to `max_concurrent_runs`. `max_concurrent
= 1` keeps one writer from overlapping itself, and `max_concurrent_runs = 1` restores one at a time
for the whole factory; neither stops two *different* writers sharing the tree.

*Declared but not built:* agents declare `access = "read-only" | "read-write"` and pipelines
`workspace = "shared" | "per-itinerary"`, and the design gives a read-only agent — and each
itinerary of a `per-itinerary` pipeline — its own git worktree. **Nothing creates one yet**: every
agent runs in its `work_dir` whatever it declares — see [`routing.md`](routing.md#5-workspace-access).
Until then, a read-only inspector reading the tree while the developer writes to it is working
against a moving target, and two instances of one writing pipeline edit the same files. `layover
explain` says so, and `validate` no longer treats `per-itinerary` as isolation when it warns about
overlapping schedules. What an implementation must settle is in the open questions in
[`decisions.md`](decisions.md#still-open).

*Unresolved even once built:* **two concurrent read-write agents remain unsafe.** The route map
does not prevent that shape, and the load-time check only looks *within* one route's fan-out. A
sequential hand-off between two writers — a developer sending finished work to a publisher that
commits it — is not flagged at all, even though the sender's process may still be alive when the
receiver starts. Nothing today guarantees the sender has exited.

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
[`CostSource`] of `reported`, `copilot_credits`, `rate_card` or `unreported`; every total in the
ledger reports the *weakest* source that fed it, so a mostly-measured figure still reads as an
estimate; and `measured_share` says what fraction of a window's runs were measured — on the costs
API, on the dashboard as "n of m runs reported nothing", and in `layover doctor`, which names the
runner responsible. A rate card can price a run that reported tokens but no dollars — labelled as
an estimate, never folded in as a measurement.

*Still unresolved:* none of this makes an unreporting runner report. The deterministic run cap is
what actually holds, and an operator has to look at `measured_share` to know whether Fuel is
metering or merely appearing to.

Copilot CLI is the sharpest case and, it turns out, not a runner that reports nothing: its JSON
stream carries `session.usage_checkpoint` events with a running total of AI units. Since it is
priced from those — `copilot_credits`, at `[copilot] usd_per_credit` — a Copilot run that reaches
its first checkpoint is metered. One that is killed before it does is still `unreported`, and so is
one whose last checkpoint cannot be read. Two things this rests on can be wrong and are worth
knowing: that one AI unit is one AI credit, which comes from the CLI's own text labelling them "AI
Credits" rather than from a documented contract; and that a credit costs a cent, which is GitHub's
published price and not necessarily what a given account pays. Both are named — the source is
distinct and the rate is a setting — so a mispriced run can be found and the rate corrected. A
runner that prints tokens and no dollars — Codex — is estimated from the factory's rate card where
it has one for the run's model: `rate_card`, shown but never debited, so the rails stay where the
run cap and the timeout put them.

### 5. Spend that no per-chain budget can see

**Severity: high. Mitigated.**

Fuel is per itinerary. A scheduled pipeline mints a fresh itinerary — and a fresh Fuel budget — on
every tick, so an hourly pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day while every
chain stays perfectly inside its rail.

*Mitigation:* the **Reserve**, a rolling-window ceiling across every itinerary, checked before
every run starts from the measured spend in history. It rolls rather than resetting daily, because
a calendar bucket can be spent twice across midnight and needs a timezone to decide when midnight
is. `layover validate` warns when a factory has a scheduled pipeline and no Reserve, or a Reserve
too small to fund one run of it. A refused run is recorded as `halted` with the reason, and
`layover doctor` reports refusals and an exhausted Reserve.

Until the Copilot credit change the Reserve was configured, validated and drawn on the dashboard
but never checked by the Tower, so this mitigation was claimed and did not exist.

A workflow may set its own `fuel_usd`, `max_runs` and `max_hops` (1.12.0). That makes the arithmetic
above per workflow — an hourly sweep at its own `fuel_usd = 8` permits $192 a day whatever
`[defaults]` says — and it is no route around the Reserve, which no workflow can set.

*Residual:* the Reserve is only as good as the cost figures feeding it, so risk 4 applies here
too — an unreporting runner spends against a Reserve that never decrements. It cannot see runs in
flight (risk 15). And it fails open when history cannot be read, trading a rail that holds through
a disk error for a factory that does not stop on one.

### 6. Agents rewriting their own static context

**Severity: medium.**

Agents given full write access to a workspace can edit that repository's `AGENTS.md` — the very
file shaping their behaviour. That is a drift loop with no human in it.

*Mitigation:* the Tower should treat instruction files as protected paths by default, overridable
in config. Note this applies to *users'* repositories; it does not apply here, because no factory
targets this repo.

### 7. Transcripts are readable through the dashboard

**Severity: low.**

`GET /runs/:id/stream` renders any run's transcript — a running one live, a finished one in full —
so whatever an agent's tools printed reaches anyone who can reach the dashboard: file contents,
command output, an API's response. Credential-shaped strings are masked as they are in a failure
detail, which catches tokens and keys with a recognisable shape and not a password printed bare.

*Mitigation:* the dashboard requires its token by default, the same one every other route needs;
`--no-auth` is for a machine only you can reach. Tool output is cut to its first lines, and the
whole transcript stays in the run's Hangar, which only the factory's own user can read.

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

### 15. Budgets are metered after the fact, not reserved before it

**What could happen.** `Reserve::authorize` is given no proposed liability and tracks nothing
in flight. With a `$40` Reserve and four Slots, four runs each costing `$20` all authorise while
recorded spend is still zero, and the factory ends `$40` over. Fuel has the same shape: a run
admitted with one cent remaining may spend a hundred dollars before its completion report arrives.

Parallel runs make this real rather than theoretical: a fan-out's runs are admitted together
against the Fuel left before any of them has reported, and `tests/parallel.rs` pins that overshoot
down so it cannot grow unnoticed.

**Why it is not fixed.** A reservation needs a *worst-case estimate* before a run starts, and the
only honest source for that is a rate card and a token ceiling the Tower controls. Guessing a
ceiling would either throttle real work or provide false assurance.

**Mitigation until then.** Slots bound how many runs can be simultaneously overshooting, so the
overshoot is bounded by `max_concurrent_runs × worst single run` rather than being open-ended.
Size the Reserve with that headroom in mind.

### 16. A rail bounds Layover's own runs, not what a child does inside one

**What could happen.** Hops, Fuel, Slots and the route map govern Tower-mediated flights and runs.
They say nothing about what a child process does once started: it runs as the same OS user, the
Copilot preset passes `--allow-all-tools --allow-all-paths --allow-all-urls` — an unattended run
cannot answer a permission prompt — and MCP wiring grants a whole server rather than named tools. A compromised reviewer can use a write-capable MCP tool to open a pull request directly,
bypassing the publisher route and its `recovery = "manual"` policy entirely — and consuming no
Hops, because none of it is a flight.

**Why it is not fixed.** Real containment means a separate OS identity or a sandbox, plus per-tool
MCP allowlists. Both belong with process supervision.

**What this means for reading the rails.** They bound *the shape of the factory*, not the
authority of an agent. An agent given a write-capable tool has that authority whatever the route
map says. Grant tools as narrowly as the runner allows: on a preset, every restriction is a
`--deny-tool` or `--deny-url` a runner, an agent or a workflow writes in `args`, and those only
ever add up, so a rule written on the runner cannot be dropped by an agent's or a workflow's. A
runner that wants an allow-list instead writes its `command` out.

### 17. External effects have no idempotency key

**What could happen.** A layover is picked up by reading a `Booked` record, starting an itinerary,
and later amending it to `Resumed`. A crash between the second and third steps leaves it `Booked`,
so the next sweep starts a second itinerary for the same work — and both are *initial* runs, so
`recovery = "manual"` never applies. Automatic recovery has the mirror problem: `ChildState::Gone`
proves the local process ended, never that the pull request it was opening did not succeed. A
restart after the model service failed has it worse, not better: that run's tools ran right up to
the call that failed, so whatever it was doing to the world very likely happened.

**Why it is not fixed.** Exactly-once external effects need a durable claim with a lease, and a
stable operation identity carried across recovery, layover pickup and spawn. That is supervisor
work, and doing half of it would be worse than none.

**Mitigation until then.** `recovery = "manual"` on the one irreversible agent in the reference
factory, and the handover text telling a resumed run to check whether the earlier one already did
the thing — after a model-service failure, that its tools may well have acted. Both rely on the
agent looking, which is weaker than a key. The one case `manual` does not stop is the safe one: a
run the model service ended before it started any tool, which did nothing to repeat; a `manual` run
that had started one is left, and the Tower files a help request for it.

### 18. A runner that under-reports its cost defeats both money rails

**What could happen.** Fuel and the Reserve are debited from a figure the child CLI prints about
itself. A runner that reports a plausible-looking `$0.01` for a `$4` run, with nothing beside it to
check it against, is believed. Both money rails then under-count by the same factor, and the only
rail left is `max_runs`, which counts invocations rather than money.

Spawn compounds it: each spawned itinerary is minted with fresh Fuel and a fresh run cap, so
generation depth bounds how deep spawning goes and nothing bounds how wide. The Reserve is meant
to be the factory-wide backstop, and it reads the same untrusted number.

**What is in place.** Negative, `NaN` and infinite figures are ignored; `$0` beside real tokens is
silence rather than a measurement; and a figure more than an order of magnitude below what its own
token counts imply is treated as `unreported` rather than believed — an unreported run downgrades
every total it is in, so the dashboard says "this is a floor" instead of showing a precise-looking
lie. `CostSource` records where every figure came from and the weakest source wins.

**What is still true.** A figure printed with no token counts beside it has nothing to be checked
against, and is believed — and Copilot CLI prints credits and no token counts at all, so a Copilot
run's `copilot_credits` figure is believed the same way. `max_runs` needs no cooperation from the
child and remains the honest rail.

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
confirm a learning, it is presented as a claim, and a person can drop one from the dashboard.
Those numbers are now the factory's (`[learnings]`): raising `provisional_runs` or `max_chars`
widens the foothold,
and lowering `confirm_after` shortens the way to permanence — for a wrong learning as much as a
right one. The defaults are the ones this risk was assessed against.

An approval queue is still rejected: it puts a human in a loop the factory exists to keep them out
of, and an earlier agent factory's queue held 88 learnings after 22 days with none ever approved.

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

### 21. An HTTP MCP server declared with `env_from` receives no credential

**What could happen.** `env_from` on a `url` server puts the named variable into the agent CLI's
environment, where a remote server cannot see it. There is no way to say "send this as the
`Authorization` header", so a server like the reference factory's `ado` is reached
unauthenticated. The agent then either fails against it or — worse — works against whatever an
anonymous caller is allowed to see and reports that as the whole picture.

**What was done.** Nothing yet, deliberately: guessing a header shape per server would write a
convention into the configuration format that nobody chose. Every declared server now reaches the
run, and its credentials are named rather than written (see the decision log), so the gap is only
the missing header mapping. The documentation says plainly that `env_from` does nothing for a `url`
server.

**What is proposed.** Open question 7 in [`decisions.md`](decisions.md#still-open): a
`headers_from` table of header templates that name variables, expanded by the CLI from its own
environment, so the secret is still never written down.

### 22. Workflows that share agents share more than routes

**Severity: high before scoping; medium with it. Partially mitigated.**

**What could happen.** A factory's workflows share agents — a review sweep and a build workflow
both use the same reviewer — and with one route map for the whole factory, every chain could use
every route. A reviewer reading untrusted pull request text in the sweep had the build workflow's
route to the agent that pushes code, because the build workflow needed it for self-review. Only a
sentence in a prompt kept the two apart, and a prompt is exactly what an injection overrides.

**What was done.** Routes can be scoped to pipelines ([`routing.md`](routing.md#8-scoping-routes-to-workflows)).
A chain may use only global routes and those scoped to its own pipeline; a spawned chain inherits
its pipeline; a resumed layover is held to what the chain that booked it could reach, so setting
work down cannot carry a chain into another workflow. The pipeline comes from the Tower's record
and queued work, never from a tool call. The route check refuses the edge whatever the prompt says.

**What is still open.**
- **Scoping is opt-in.** A factory that scopes nothing has exactly the isolation it had before —
  none — and a global route is usable by every workflow.
- **An agent's memory, its learnings and the Logbook are per agent or per factory, not per
  workflow.** A reviewer that notes something in `memory.md` during the sweep reads it back during
  the build. That is a channel an injection can write to and a later chain in another workflow will
  be handed; it carries text, not permissions, but text is what an injection is made of.
- **The working directory is shared**, whatever the routes say — see risk 2.
- **The Tower trusts its own files.** A chain's pipeline is read from the queue under `.layover/`.
  Anything able to rewrite that file can relabel queued work; an agent is not meant to be able to,
  but nothing stops an agent whose `work_dir` contains the factory's state directory.

### 23. Two processes running one factory's queue

**What could happen.** Locks on the queue, the learnings and each Hangar's memory are held within
one process. `layover run` beside a `serve`, or two `serve`s over one directory, each take a flight
off the queue by reading it and writing it back, and can both believe they took the same one — so
it runs twice.

**Why it is not fixed.** A lock across processes on every journal file is a different mechanism from
the one runs in parallel needed, and running one factory from two processes is not a supported
shape: `layover serve --watch-only` is the way to point a second window at a factory.

**Mitigation until then.** A second `serve` on a factory another is serving refuses to start: each
holds `.layover/serving.lock` for as long as it lives. `layover run` takes no such lock, and beside a
`serve` can still race it for queued work. Each Tower's claim on its live runs is a lock the
operating system holds across processes, so a second Tower never stops or restarts the first one's
runs. Do not run `layover run` on a factory a Tower is serving.

### 24. On Linux and macOS, ending a run ends the agent CLI, not what it started

**What could happen.** A run that times out or meets a Ground Stop is killed. On Windows the whole
tree goes, because `taskkill /T` follows the parent chain. On Unix the Tower kills the process it
spawned and, if that process leads a process group, the group; an agent CLI that does not start one
leaves its language servers, MCP servers and test runs behind, reparented and still running.

**Why it is not fixed.** Starting every run as the leader of its own process group would let the
Tower end the whole tree, as `kill_tree` was always meant to. It would also take the runs out of the
terminal's foreground group, so Ctrl+C on a `serve` or `run` in a terminal would no longer reach
them — they would run on, cut off, until the next Tower settled them. That is a change to what
stopping the Tower means, and it deserves a decision of its own rather than a line in a fix.

**Mitigation until then.** Most children end when their parent does, because the pipe they read from
closes. A Tower restart settles the agent CLI itself. `timeout_sec` still ends the run's process,
which is what `doctor`, the slots and the Fuel meter all count.

### 25. Telling a model-service failure apart depends on the CLI's event names

**What could happen.** A failed run is restarted only when its CLI's events say the model service
failed its last call: for Copilot CLI, `model.call_final_result` with a `result` other than
`success`, `session.error` with `errorType` `query`, or the line of text it prints at start-up for a
model that is not available — `Error: Model "…" from --model flag is not available.` — and no
`tool.execution_start` or successful call after it. Those are names, values and words a CLI release
can change; the words are kept in one list, `provider::MODEL_UNAVAILABLE`, with a test for each. Renamed, a provider error reads
as the run's own failure and is not restarted — the behaviour before restarts existed, and the chain
shows as `failed`. Misread the other way, a run that failed its own work is restarted, and fails
again.

**Why it is not fixed.** The alternative — matching text in the output — would also match an agent
quoting an error, which is worse. Only the CLI knows its events, and it does not promise them.

**Mitigation until then.** A misread restart is bounded like every other: `max_recovery_attempts`
per piece of work, each restart an ordinary run against Hops, Fuel, `max_runs` and the Reserve,
`recovery = "manual"` honoured, and a Ground Stop forbidding it. The shapes are pinned by tests built
from real Copilot CLI 1.0.97 transcripts. A runner that wants none of it sets
`provider_errors = { read = "none" }`; one for a CLI that changed declares the events itself.
`manual` lets a run restart only when it started no tool, which reads an *absence* — the weaker
direction — so a tool counts as started when either its start or its completion is in the
transcript, and one renamed event cannot make a run that acted look untouched.

### 30. An agent CLI run in your own home has your own tools

**What could happen.** A run keeps the home directory, so its CLI reads the user's own
configuration: the plugins, skills and MCP servers installed for working at the keyboard. An
unattended agent can then call a tool `layover.toml` never granted it — one that posts to a chat or
a tracker as that person — and nothing in the factory's definition says so. Every run also lands in
the user's session history, where hundreds a day bury the user's own.

**Why it is accepted.** The home directory is how a CLI finds its sign-in; taking it away breaks
every run. Making a home of the factory's own would need Layover to sign each CLI in, which it
cannot do or check.

**Mitigation.** The documentation recommends a CLI home of the factory's own — `COPILOT_HOME`,
`CLAUDE_CONFIG_DIR`, `CODEX_HOME` — with no plugins and no global MCP servers, forwarded with
`env_from`, which refuses a run when it is not set. An agent's MCP servers are then exactly the
ones declared for it.
