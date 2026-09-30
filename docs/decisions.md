# Decisions

Why Layover is built the way it is, and what is still undecided.

This file exists because there is no ADR trail: an agent opening this repository starts fresh and
cannot recover intent from a diff. The reasoning has to be written down somewhere, and one place
is better than two — a decision recorded in two documents is a decision that will be revised in
one of them.

> **Settled questions are not listed separately.** Each one is a decision, and the decision log
> below carries it *with its reasoning*, which is the more useful record. A second index of the
> same answers only offered a second thing to forget to update.

System design lives in [`architecture.md`](architecture.md); route map semantics in
[`routing.md`](routing.md); known hazards in [`risks.md`](risks.md).

---

## 13. Decision log

Rationale for the decisions that are not self-evident. This section exists because there is no
separate ADR trail: an agent opening this repository starts fresh and cannot recover intent from
a diff. Following the project's own philosophy, the reasoning has to be written down somewhere.

**Why supervise CLIs instead of calling LLMs directly.** The agent CLIs already solve tool use,
sandboxing, context management and provider auth. Reimplementing that would be the whole project.
Supervising them means Layover's scope stays orchestration, and it inherits improvements to those
CLIs for free. The cost is that agents become opaque processes, which is what forces §4.2.

**Why MCP is the control channel.** All three target CLIs speak MCP natively, so agents gain the
ability to message each other with no adapter code and no bespoke protocol. This also answered the
open interop question: Layover does not need to pick between A2A, ACP or a greenfield protocol,
because the control channel and the integration standard turned out to be the same problem.

**Why sending a message is what starts an agent.** Two operations — spawn and send — would need
two permission models over the same graph, and would allow the incoherent state of an agent
spawned with nothing to do. Unifying them makes the route map the single authority over both
communication and lifecycle.

**Why runs are fresh rather than resumed.** Resumed sessions grow without bound, make cost
unpredictable, and hide what an agent actually knows inside an opaque transcript. Fresh runs force
memory to be deliberate and inspectable. See §4.1.

**Why shared memory is Markdown rather than SQLite.** The Logbook is meant to be read by humans
during an incident and edited by hand when an agent records something wrong. A database would be
more robust and less useful. Write safety is recovered by serializing writes through the Tower
rather than by the storage format.

**Why workspace contention is only partly mediated.** Full locking or per-run worktrees for every
agent are real work and would delay proving the core concept. The decision was that read-only
agents get a worktree snapshot, which would make the common fan-out shape safe for free; two
concurrent read-write agents remain unsafe and the risk is recorded in
[`risks.md`](risks.md#2-shared-workspace-contention) rather than forgotten. *The snapshot is not
built yet* — every agent runs in its `work_dir` — and what building it has to settle is open
question 8 below.

**Why fan-in is a join on the receiving node rather than a pipeline definition.** Two edges into
one agent would otherwise fire it twice, on the first arrival rather than the last — duplicate
side effects, silently. A pipeline DSL would fix that by dictating sequence, but it would also
turn the route map from a permission graph into an execution graph and take routing decisions
away from agents. Declaring the barrier on the *receiver* keeps senders free and the mesh
emergent: nobody is told what to do next, a joined agent simply cannot be woken by one input
alone. It also removed the need for blocking `request_response`, because the Tower parks flights
instead of parking processes.

**Why a barrier guards its upstreams rather than its agent.** A join could plausibly mean "this
agent may not run until these inputs arrive". It does not; it means "these inputs reach this agent
together". The difference only shows up when a joined agent is also reachable another way, and
then it decides whether the design works at all: an entry agent that collects results from the
helpers it dispatches would, under the stricter reading, park its own human trigger while waiting
for agents that cannot run until it has been triggered. Deadlock on the first flight. Scoping the
barrier to its declared upstreams also removes the need for an intermediary gate agent in a review
loop — verdicts rendezvous directly on the agent that produced the work, which then decides for
itself whether to loop or move on. The cost is that a run may wake holding less than everything
in flight for it, which is why sender identity is mandatory.

**Why optionality lives in prompts rather than in the route map.** `join = "all"` waits for every
*declared* upstream, so an agent that consults a specialist only when the work calls for one would
strand its own rendezvous — the barrier waits for an agent that was never asked, and the itinerary
stalls on the happy path. Making the barrier wait only for upstreams actually dispatched is the
real fix, and it is deferred rather than dismissed: it requires the Tower to observe dispatch, and
it races, because a fast upstream could release the barrier before its sibling is dispatched at
all. Until then the rule is to dispatch everyone every time and let an idle specialist answer
"nothing to add" — a cheap read-only run in exchange for a barrier that can always be satisfied.

**Why Hops is checked at load time but only weakly.** An agent further from an entry point than
`max_hops` can carry is configured, appears live in the route map, and never runs — worth catching
before the first flight. Plain reachability is not enough on its own, though: a `join = "all"`
target looks close when *any* upstream is close, while it actually waits for the last, so a second
check requires every upstream to be able to afford the flight into the barrier. Both measure
shortest paths, and the budget is really consumed by *loops*: a two-agent review cycle costs two
hops per turn, so a route map three flights deep can need twenty to be useful. Nothing static can
know how many times a loop will turn, so the checks deliberately prove only the negative. The
reference scenario carries the arithmetic instead, and a regression test pins it, because the
default `max_hops = 8` permits that factory's happy path and not one round of rework.

**Why the factory never targets Layover's own source.** It removes an entire class of hazard —
agents editing the supervisor that is running them — and makes it safe to give agents full write
access inside their workspace. The cost is losing the most persuasive dogfooding demo.

**Why the HTTP server is generated from a specification rather than described by one.** A
hand-written server with a hand-written OpenAPI document beside it has two sources of truth and no
force keeping them equal; the document rots first and quietly, because nothing breaks when it
does. Generating the server makes the document load-bearing: a drifted spec is a failing build,
not a stale page. The cost is a generator to maintain, and it is kept cheap by supporting only the
subset this API uses and erroring loudly on anything else — a generator that silently ignored
part of the specification would recreate exactly the problem it was meant to solve.

**Why pipelines are separate from `entry = true`.** They are different things. `entry` is a bare
permission: a human may poke this agent. A pipeline is a *named trigger* that also carries a
schedule and the flags a run is parameterised by. Folding them together would either force every
one-off entry point to declare a pipeline, or leave schedules and flags with nowhere to live.
Keeping pipelines thin — an entry agent, a trigger, some booleans — is what stops them becoming
the pipeline engine the route map deliberately is not.

**Why a schedule may not fire more than once a minute.** A schedule is the only part of Layover
that starts work with nobody present, and runs are reentrant: a schedule that outruns its own work
does not queue, it accumulates concurrent copies. Six-field cron expressions are refused for the
same reason — a seconds field can schedule work faster than a run can finish, which is a fork bomb
with a clock attached. The floor is a minute because that is the finest a five-field expression
can state, so `every` and `cron` agree about what is possible.

**Why prompt composition is conditional rather than templated.** Prompts wanted to vary — a tester
that sometimes runs a remote suite is the same agent with one extra paragraph — and the obvious
answer is string interpolation. Booleans chosen at trigger time are deliberately less powerful:
every possible prompt is a file somebody can read and review, and `layover prompt` can render any
of them exactly as a run would receive it. A templating language would make prompts programs, and
the thing an agent is told would stop being reviewable. The rails around it all exist because the
failure mode is silent: an undeclared flag is an error rather than false, because treating it as
false would let a typo delete a section of instructions without anyone noticing.

**Why a run is told who sent its flight, and in which words.** The prompt guide and the routing
notes told authors that sender identity is how an agent tells its inputs apart, but the payload
held only the body, so factories wrote `FROM <agent>` into every body by hand — an identity the
sender asserts, which is the thing §4.2 of the architecture exists to rule out. The Tower already
records who sent each flight, so the payload states it above the body. It says what *kind* of
sender it was, because "fix the retry policy" means one thing from a reviewer and another from a
clock nobody is watching; that needed `Origin` to stop recording a schedule and a resumed layover as
a person. A released join states nothing there: its body already labels each flight it folds
together, and naming one sender above them would be wrong about the rest.

**Why a schedule is stored as a person with a note beside it.** The richer `Origin` could not simply
be written as new variants. A 1.0.0 binary reading the queue drops a line it cannot parse and then
rewrites the queue without it, so a downgrade would silently lose scheduled and resumed work. The
on-disk flight keeps the two senders 1.0.0 knows and puts the rest in an optional `via` field it
ignores: a downgrade loses a label, never the work, and the state directory's layout — whose bump
would be a breaking change — stays as it is.

**Why prompt flags are checked per entry point rather than per factory.** A run carries the flags
of the one pipeline that triggered it, never the union of every pipeline in the factory. Checking
a prompt against that union looks equivalent and is not: a second pipeline that reaches the same
agent without declaring the flag passes validation and then fails at composition time, hours
later, with nobody watching. So validation walks reachability from *each* entry point and requires
every flag a reachable prompt tests to be declared *there*. The same rule covers a bare
`entry = true` agent, which supplies no flags at all — any conditional prompt downstream of one is
unreachable in practice, and saying so at load time is the whole point.

**Why a chain's flags travel with its work.** A flag is a choice somebody makes when they trigger
work, and it was being honoured for nothing: the trigger stored it, and every run then composed
its prompt from the pipeline's defaults, so `layover prompt --flag` previewed text no run received.
Remembering the flags per itinerary in the Tower fixed the first run and nothing after a restart,
because that memory starts empty. So each queued flight carries its chain's pipeline and flags,
the way the first flight always did, and the Tower records them from whichever flight it sees
first. The values come from the Tower's record of the run — the token's session — and never from
the agent, because an agent that could set a flag could switch on the section of its instructions
that lets it publish.

**Why a spawned chain belongs to the pipeline that spawned it.** A spawn edge gives a chain fresh
Hops, Fuel and a run cap; it was also giving it no pipeline, so its prompts were composed from
every pipeline's defaults merged — the last declaration winning on a clash — and its runs and cost
belonged to no workflow. The first contradicted the operator, the second contradicted the rule
that workflow membership crosses spawn edges. A spawned chain now inherits both the flags and the
pipeline of the chain that spawned it, and prompt-flag validation follows spawn edges for the same
reason: the spawned agent is composed from the spawning entry point's declarations.

**Why a resumed layover takes its flag values from the chain that booked it.** The follow-up is the
same work, and the choices about that work — run the end-to-end suite, keep the pull request a
draft — were made when it was triggered; resuming from the resuming pipeline's defaults undid them
days later with nobody watching. But the *set* of flags stays the resuming pipeline's, because
validation proves the prompts reachable from there test only what it declares; composing from the
booking chain's set instead could fail at runtime on a factory that passed `validate`. So the
layover records the booking chain's flags, and the resumed run takes those values for the flags
the resuming pipeline declares and its defaults for the rest.

**Why the overlap warning reads a cron expression's minute field.** A schedule that outruns its
own work accumulates concurrent runs rather than queueing, so the check needs a lower bound on how
often a pipeline can fire. `every` states it outright. Cron does not, but the common footguns —
`* * * * *` and `*/5 * * * *` — state it in the minute field, and reading only those two forms is
enough to catch them. Lists and ranges are deliberately left alone: a wrong lower bound produces a
warning that is not true, and a validator that cries wolf is one people stop reading.

**Why MCP servers are declared per agent rather than left to each CLI.** A telemetry agent needs a
Kusto endpoint and a publisher needs an issue tracker, and configuring those in each CLI's own
settings puts them somewhere the route map cannot see, nothing validates, and no reviewer reads.
Declaring them in `layover.toml` makes an agent's reach part of its definition. The cost is that
config now sits next to credentials, which is why `env` and `env_from` are separate: `env` is for
values safe in a committed file, `env_from` names variables forwarded from the Tower's own
environment, and validation *refuses* a literal whose name looks like a credential. A warning
would not do — the failure is a key in git history, which is not undone by noticing later.

**Why a declared server's credential is named in the run's configuration rather than written.**
Declaring servers in `layover.toml` only helps if they reach the run, and the one channel into a
CLI's MCP setup is the configuration file the Tower hands it — which lives in the run's Hangar
under `.layover/`. Writing an `env_from` value there would move the secret from git history to a
directory anything on the machine can read. The Tower already puts the value in the child's
environment, and every supported dialect can say "read it from there": `${NAME}` in the JSON that
Claude Code and Copilot CLI read, `env_vars = ["NAME"]` in Codex's TOML. So the file names the
variable and never holds it. Verified against Copilot CLI 1.0.88, which also passes its whole
environment to the stdio servers it starts; Codex passes only an allow-list, which is why naming
the variable is not optional there.

**Why parallel instances needed only a workspace change.** "One pipeline instance per pull
request" sounds like it needs an instance concept, and it does not: every trigger already mints
its own itinerary with its own Hops, Fuel, barriers and flags, because barriers are keyed by
`(itinerary_id, to_agent)` and Fuel is per chain. The single thing instances actually shared was
the working directory. `workspace = "per-itinerary"` is to give each a git worktree, and the rest
was already true. Left as opt-in because a worktree per itinerary costs disk and setup time, and a
single-instance pipeline wants neither. *Not built yet*: the setting is accepted and does nothing,
which `layover explain` says; see open question 8.

**Why autostart generates rather than installs.** A lights-out factory that stops at every reboot
is not lights-out, so the Tower has to survive one. But registering a service writes to the
machine, and the artefact is exactly the kind of thing a person should read before it runs at
every logon. So `layover autostart` renders the platform's own file — Scheduled Task, launchd
agent, systemd user unit — and prints the one command that registers it. It also refuses to write
anything for a factory that does not load, because a service failing at every logon is worse than
no service. All three artefacts install into the logged-in user's session and never machine-wide:
the Tower spawns agents using *that user's* credentials, git identity and workspace, and a system
service would have none of them or would run as root with all of them.

**Why recovery and steering are the same mechanism.** They arrived as different requests — survive
a VM restart, and let a human redirect work in progress — and both looked like they needed a
process kept alive: resume the conversation, or pipe input into the running child. Either would
have made `resident` the normal case and brought back the reentrancy hazard that fresh runs
retired. Starting a *new* run seeded with a **handover** answers both, because what the second run
actually needs is not the first run's process but its context. Nothing is resumed, no session is
held open, and the locked "fresh runs" decision survives intact: a run is still a clean slate
process, it simply opens with more to read.

**Why recovery is bounded like every other rail.** A restarted run spends a hop, debits Fuel,
counts against the run cap and draws on the Reserve, and `max_recovery_attempts` bounds it on top
of all that. A crash loop that restarts itself forever is a fork bomb that looks like resilience.
A Ground Stop is never restarted through, because a factory that restarts past its own kill switch
is not one anybody can stop.

**Why Layover does not guess which agents are unsafe to restart.** Repeating work is harmless for
an agent that reads and reports, and dangerous for one that opened a pull request — a run
interrupted after pushing a branch but before recording it would, on restart, open a second. The
tempting check is "terminal and read-write", and it is a topological guess at a semantic property:
it fires on plenty of factories where repeating is fine, and a validator that cries wolf is one
people stop reading. So the mitigation sits in two better places. The handover text *tells the
agent* to check whether the earlier run already did the thing, which is the only party that can
actually look; and `recovery = "manual"` stops the restart outright for the steps where checking
is not good enough. The reference factory sets it on exactly one agent.

**Why Apache-2.0 alone rather than the Rust-typical `MIT OR Apache-2.0`.** Rust libraries dual
licence so that GPLv2-only projects can consume them, since Apache-2.0 is incompatible with
GPLv2. That reasoning is about libraries, and Layover is an application: people run the binary,
they do not link `layover-core` into something else. What Apache-2.0 adds over MIT is an explicit
patent grant, which is worth more here than compatibility with a licence nobody consuming a
supervisor is likely to be bound by. Revisit if `layover-core` is ever published as a crate meant
to be depended on — at that point the dual licence becomes the right default again.

**Why the milestone is named rather than numbered.** "v0.1" used to mean the release where a
factory first actually runs, while the crates were already at 0.8.0 — so the project appeared to
be both far behind and far ahead of itself, and a stranger could not tell which. Crate versions
now track what is built and the milestone has a name instead of a number. A goal spelled like a
version will be read as a version.

**Why the declared MSRV is the pinned toolchain.** `rust-version` said 1.85 while
`rust-toolchain.toml` pinned 1.98.1, which is the only compiler CI ever runs — so the older
number was an untested claim, and thirteen minor versions is a long way to be wrong. The choice
was to test the claim or to stop making it. Testing it means a second CI job and a second
toolchain download on every push to protect users nobody has yet; stating the version that is
actually exercised costs nothing and is true. If somebody turns up needing an older compiler,
that is the moment to find out how far back it really builds.

**Why documentation upkeep is in `AGENTS.md` rather than in review.** `docs/` is normative and
hand-maintained, and an agent that trusts a stale document makes confident wrong changes. Review
catches that only if a human remembers to look. So the standing instruction is that every change
carries its documentation, `AGENTS.md` names which file goes with which kind of change, and
`verify` enforces the mechanical part — links that resolve, generated code that matches, examples
that still validate. No tool can check whether a paragraph is still true, which is precisely why
the instruction has to be standing rather than requested.

**Why there is a Reserve as well as Fuel.** Fuel bounds one itinerary, and that turned out not to
bound the factory. A scheduled pipeline mints a *fresh* itinerary — and a fresh Fuel budget — on
every tick, so an hourly pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day with every
individual chain sitting perfectly inside its rail. The rail was real and the arithmetic still
ran away. The Reserve is the missing axis: a ceiling on total spend that no per-chain budget can
reset.

**Why the Reserve rolls instead of resetting daily.** A daily cap sounds simpler and is worse
twice over. Midnight doubles it — spend the cap at 23:59 and the bucket resets a minute later, so
"$50 a day" permits $100 in two minutes. And a day needs a timezone: a sibling project's daily
gate bucketed by UTC while its ledger bucketed by local time, so between local midnight and the
UTC offset the gate read the wrong day's total and let spending through. "At most $50 in any
rolling 24 hours" has no midnight, no timezone and no daylight-saving edge, and is strictly
stricter.

**Why a cost figure carries where it came from.** A number can be reported by the runner, derived
by Layover from token counts and a rate card, or absent entirely, and those are not
interchangeable. Collapsing them is how a budget quietly becomes fiction: the same sibling project
priced runs from a hand-maintained table and ran **2.7× over actual** — billing one model at `$75`
per million output tokens where the provider charged `$25` — with nothing in the totals saying
"this is a guess". So `CostSource` is a field, not a comment; a total reports the *weakest* source
that fed it, and a summary that is 90% measured still reports as an estimate. For the same reason
Layover ships no rate card: prices change per provider and per context tier, and a stale table
baked into a release is precisely how the drift happens.

**Why Copilot runs are priced from their credits.** Copilot CLI prints no dollars and no token
totals, so every Copilot run was `unreported`: Fuel debited nothing, the Reserve counted nothing,
and a Copilot factory was bounded only by its run cap and its timeout. It does print its usage —
`session.usage_checkpoint` events with a running total of AI units, `totalNanoAiu` — and GitHub
publishes the price of that unit: one AI credit is one US cent. So a Copilot run is priced at the
last checkpoint's credits times `[copilot] usd_per_credit`, and recorded as its own source,
`copilot_credits`. Three alternatives were weighed and rejected:

- **Premium requests**, the unit this entry's open question once recommended. They are a flat
  model multiplier per prompt — Opus 5.5 reported 15 for a 47-minute run and for a 6-minute one
  alike — so a budget in them cannot tell a long run from a short one, which is the one thing a
  budget is for. Never priced, not even as a fallback when no checkpoint was seen.
- **A separate budget counted in credits**, beside dollars. Deterministic and needing no price,
  but a factory mixing runners would have two budgets neither of which saw the other's spend.
- **A rate card entry**, labelled an estimate. The count is not an estimate — Copilot measured it
  — and labelling it one would make every Copilot total read as a guess.

The default rate is a named constant rather than something the operator must supply, which looks
like it breaks "Layover ships no rate card" and does not: that decision is about per-model token
prices, which drift per provider and per tier; this is the published size of the unit Copilot
counts in. A factory billed at another rate overrides it, and `validate` refuses zero, negative and
non-finite rates, because zero would record every Copilot run as free *and* measured. That one AI
unit is one AI credit is an assumption, from the CLI's own text output labelling them "AI Credits";
it is why the source is named apart from `reported` — if the assumption is ever wrong, the affected
runs can be found and repriced.

`copilot_credits` is measured: it debits Fuel and draws on the Reserve like `reported`, counts
towards `measured_share`, and is not "reported nothing" to `layover doctor`. A total that includes
it has confidence `copilot_credits`, weaker than `reported` and stronger than `rate_card`, and the
dashboard names it — "3 of 5 runs priced from Copilot credits" — so the arithmetic stays visible.
Everything else is unchanged: a last checkpoint that cannot be read, reads as negative or is not an
integer makes the run `unreported` rather than priced from an earlier total; zero credits beside
premium requests is silence; and a run killed before its first checkpoint is `unreported`, and
every total built on it a lower bound.

This changes what an existing Copilot factory does: its `fuel_usd` now refuses work, and so does
its Reserve.

**Why the Reserve is checked from history, at dispatch, and nothing more yet.** The Reserve was
configured, validated and drawn on the dashboard, and nothing refused work when it ran out — the
Tower never consulted it. Pricing Copilot runs made that impossible to leave: a rail an operator
can now watch fill up has to do something when it is full. So before a run starts, after
everything about the flight itself and before it counts against the chain's run cap, the Tower adds
up the measured spend in history over the Reserve's rolling window and refuses at the cap.

History rather than an in-memory tally, because history is where every finished run's cost already
lives, it survives a restart, and it is what the dashboard's Reserve card reads — so the rail and
the figure an operator sees cannot disagree. A refusal is written into history as a `halted` run
whose detail says how much was spent and when the window frees room: a scheduled trigger refused
before it starts leaves no other trace, and "nothing is running" must not look the same as "nothing
is allowed to run". It is recorded as a measured zero rather than `unreported`, because it cost
nothing and that is certain. If history cannot be read the check fails open, as history does
everywhere else; a factory that stopped because a log file was momentarily unreadable would have
turned an observability problem into an outage.

Enforcing the Reserve also enforces its documented default, $100 in any rolling 24 hours, on a
factory that never wrote `[reserve]`; `fuel_usd = 0` is how to say unlimited. Still not built from
the first-release design: the banner, the warning at 80%, the automatically raised help request,
and a cap raised by editing `layover.toml` without restarting the Tower — it reads its
configuration once. Nor does the check see runs still in flight (risk 15).

**Why the run cap is not the same as a cost rail.** Fuel depends on runners voluntarily reporting
cost and not all of them do, so the deterministic run cap holds when Fuel cannot. What changed is
that the gap is now *countable* rather than a boolean: an itinerary records how many of its runs
went unmetered, so "three of forty" and "all forty" are distinguishable. The first is a gap; the
second means the cost rail is not running at all.

**Why Fuel is required from the first runnable release rather than deferred.** Hops was originally assumed to be the
anti-fork-bomb rail. It is not. A hop is spent per flight and branches inherit the remaining
count, so Hops caps how *deep* a chain runs and says nothing about how *wide* it spreads — a
branching factor of 3 at `max_hops = 8` permits thousands of real, paid CLI invocations from a
single trigger. Only a shared per-itinerary budget bounds that, so Fuel moved from "later" to
"required". The follow-on consequence is that Fuel may not depend on runners voluntarily
reporting cost; it needs a deterministic fallback, or the single breadth rail can vanish silently
while still appearing to be enforced.

**Why history is a directory of text files rather than a database.** History is append-only,
written once and read in whole windows — the one shape a query planner is least needed for. The
cost would have been real: `rusqlite` bundles a C library, which means a C cross-compiler for
each of the five targets Layover releases to, trading the musl and aarch64 builds for indexing
that a few megabytes of JSON does not need. Segmenting by day buys three further properties a
single growing file would not: retention becomes deleting files rather than rewriting one, a
window opens only the days inside it, and a partial write — which is what a power cut leaves —
costs one line instead of the file.

**Why the segments are UTC days while the reports are local.** Reporting windows are local
because "this month" is a local question, but a local-day *filename* shifts when the machine
changes zone or the clocks go back: two days would want the same name, or one day would be split
across two files. UTC has no such day. Reading a local window therefore opens one extra segment at
each end and filters by instant — cheap, and it cannot be wrong.

**Why a window is either rolling or calendar, in the type.** "The last 7 days" and "this month"
are asked in the same breath and are not the same kind of question: the first is 168 hours
everywhere on earth, the second begins at a midnight that depends on where the Tower is standing.
Collapsing them is how a factory spends one day's money twice — a sibling project gated on a UTC
boundary while reporting in local time, and for the hours between the two midnights the gate and
the display described different days. So `Window` distinguishes them and every resolved `Span`
carries the zone it was reckoned in, or `None` when there was nothing to reckon. The absence is as
informative as the name.

**Why the dashboard draws its own graph instead of using Mermaid.** Mermaid stays for
`layover graph`, where the output is pasted into a README and portability is the whole point. It
was rejected for the dashboard on weight: the runtime is 2.5 MB of JavaScript that would have to
be vendored into the repository and embedded in the binary to keep the page working offline. A
layered layout for twenty nodes is a few hundred lines that can be unit-tested, where asserting
on a JavaScript library's rendering could not be. Layers come from breadth-first distance rather
than longest path, because route maps are routinely cyclic — the review loop is the point of the
reference factory — and longest-path layering does not terminate on a cycle. Reusing the distance
the validator already computes also means the diagram's columns and the hop arithmetic can never
disagree.

**Why the dashboard refuses control operations instead of hiding them.** Sending a flight,
streaming a run and engaging a Ground Stop all need a supervisor that does not exist yet, and
they answer `501` rather than returning something plausible. A control that silently does nothing
is worse than a control that is not there: it is trusted once, and then relied upon at the moment
it matters.

**Why the prompt is written to stdin and never onto the command line.** The runner template
originally inlined it as `command = ["claude", "-p", "{prompt}", ...]`. Windows caps a command
line at 32,767 characters, and real agent prompts are nowhere near small enough: measured against
a sibling project's prompt trees, its review agent composes to roughly 98 KB — three times over
the limit — and its ordinary developer agent to 34 KB. Two of that project's six agents would not
have started. It is a failure mode that passes every test written against a small fixture and
appears on the first agent worth running, which is the worst shape a bug can have. That project
had already found it the hard way; its runner carries the comment "never `-p` / a shell pipe".
`{prompt}` now substitutes the *path* to the composed instructions, for CLIs that accept one.

**Why recovery requires the previous process to be confirmed gone.** `Interruption` distinguishes
what the Tower *watched* from what it merely *inferred*. A timeout or a non-zero exit means it saw
the process end. A Tower restart or a lost pipe means only that it stopped being able to see one —
and on Windows a child routinely outlives the parent that spawned it. Recovering in that state
starts a second run beside a first that never stopped, which for a publisher means two pull
requests. So those interruptions carry `ChildState::Unknown` until something checks, and
`authorize_recovery` refuses them; the run record keeps the process id so there is something to
check. A recycled process id can make a dead run look alive, which fails towards refusing to
recover — the safe direction, because stalled work is visible and duplicated work is not.

**Why learnings apply immediately instead of waiting for approval.** The obvious design puts a
human between a proposal and its use. A sibling project built exactly that — proposal format,
duplicate detection, impact ratings, a review endpoint, a dashboard queue — and after 22 days of
real operation held 88 learnings, every one still pending, none ever approved. Since only approved
learnings were injected, not one had ever reached a run: everything was built except the step that
creates the value. That is not a discipline failure but an incentive one. Approving buys a diffuse
future benefit, rejecting buys nothing, and ignoring costs nothing today, so a gate whose default
action is free gets defaulted forever. A learning here applies at once and expires after twenty of
its agent's runs, so a wrong one decays rather than compounding, and review happens by exception.
That is only defensible because run history records which learnings were live for each run, which
makes "what was it told when it did that?" answerable and revocation a single press.

**Why an echo does not count as a rediscovery.** Confirmation comes from a learning being
independently arrived at three times, which is evidence — unlike the `impact` rating, which is the
agent's own claim about its own work and therefore decides nothing. The trap is that showing a
learning to an agent contaminates the signal: repeating advice you were just given proves nothing,
and counting it would let a single fluke confirm itself within three runs. So duplicates are
suppressed while a learning is active — the same behaviour the sibling project needed, for the
opposite reason — and only a proposal arriving while the learning has lapsed increments the count.

**Why sameness is decided by containment rather than overlap.** Whether two sentences say the same
thing decides whether rediscovery is ever recognised, and it fails silently in both directions: too
strict and nothing is ever confirmed while the mechanism appears to work, too loose and two
insights merge and one is lost without trace. Word overlap is the wrong measure because real
learnings share sentence frames — "the workspace needs careful handling before publishing" and "the
manifest needs careful handling before publishing" overlap five words out of seven while being
different claims. Containment discriminates correctly: a rediscovery with an added clause is a
superset, while two different insights each carry a word the other lacks. A minimum length stops a
two-word learning matching everything, and a ceiling on elaboration keeps a substantially more
specific claim separate, which is what a refinement is.

**Why help requests are pruned and learnings are not.** Help requests are events: each happened at
a moment, is answered or not, and stops mattering. They are segmented by day and expire on the same
ninety-day horizon as run history. Learnings are state, rewritten as they are rediscovered or
revoked, and exempt from retention entirely — a confirmed learning that expired for being ninety
days old would be the one thing in the system that got worse the longer it was right.

**Why an unknown help category is refused rather than filed as `other`.** The category is optional,
because an agent that asks for help without classifying it has still asked, and refusing would
lose the one message meant to reach a person. But a value that is *not* a category is a mistake
somebody should see: filed as `other`, it would vanish from the dashboard's filter — the thing the
category exists for — and nobody would learn that the prompt names a category Layover does not
have. The refusal lists the six, so the agent's second attempt is right.

**Why Slots queue where every other rail refuses.** Hops bounds depth, Fuel and the Reserve bound
money, the run cap bounds a chain's total — and none of them bounds how many agent CLIs are alive
at one instant, which is the number that takes a machine down. The gap surfaced by writing a real
pipeline: a scanner dispatching one reviewer per pull request assigned to you has a width nobody
knows until it looks, and fifty assigned pull requests meant fifty simultaneous processes with
every rail satisfied. Slots queue rather than refusing because every other rail protects a budget,
and money spent is gone, whereas a machine that is busy now will not be busy in a minute. Refusing
would turn "review twelve pull requests" into "review four and silently drop eight", which is the
worst available reading of a concurrency limit.

**Why one coordinator decides and many threads wait.** Until 1.4.0 the Tower ran one agent at a
time while `max_concurrent_runs` was parsed and read nowhere: an Eagle Eye sweep of ten hour-long
reviews took ten hours, and a 14:00 schedule fired at 14:23 because the clock waited for the run in
front of it. The fix keeps everything that decides *whether* work may start on one thread —
unqueueing, barriers, admission against Hops, Fuel and the run cap — and moves only the waiting,
which is where all the time goes, onto a thread per run. So the guarantees the sequential loop gave
hold without a lock around them: a flight leaves the queue before it runs and is never taken twice,
a barrier sees its arrivals one at a time, and a chain's rails are charged in the order its flights
were admitted. A chain's ledger is held only while it is charged — admission before the spawn, the
Fuel debit after the exit — so runs of one fan-out alive at once each see what the last left.

**Why the clock ticks on the dispatcher's thread.** A schedule skips a tick while its last wave is
queued *or* running, and a flight moves from one to the other as it starts. A clock on a thread of
its own could look at both between those moments, see neither, and start a second copy of work
already under way. Ticked from the dispatcher between one step and the next, nothing moves while it
looks. The dispatcher looks for new work every quarter second while runs are alive, so schedules
still fire on time.

**Why the next flight is the oldest one that can start.** First in, first out, among flights whose
agent is below its own `max_concurrent`. A flight for an agent at its cap waits where it is, and
flights behind it for other agents go ahead: waiting for the one agent is what the cap asks for,
and holding the whole factory behind a single Teams sender is not.

**Why the slot count comes from the runs alive rather than from `Slots`.** `Slots` keeps a waiting
line of run identifiers, but a queued flight has no run yet, and the line work actually waits in is
the journal's queue — durable, and read back after a restart. A second line in memory would be a
second source of truth that dies with the process. So the dispatcher counts the runs it holds alive
against the limit and takes the next eligible flight off the journal's queue.

**Why a restarting Tower stops a run it finds still alive.** Such a run is cut off: its MCP endpoint
and token died with the Tower that minted them, so nothing it sends, reports or books can arrive,
and left alone it keeps spending on work nobody will receive. The alternatives were weighed: letting
it run to completion in a held slot, or doing that only for agents whose `recovery` is `manual`.
Both leave the chain dead-ended — its output has nowhere to go — and pay for work that is thrown
away. Stopping it is also what makes it *confirmed* gone, which recovery requires; the restart is
then decided by the agent's `recovery` policy and `max_recovery_attempts` as for any interruption,
and is told to check before repeating a side effect. Decided with the maintainer on 30 September
2026.

**Why each Tower holds a lock rather than recording its process identifier.** A Tower settling runs
must leave alone those another living Tower is watching — `layover run` beside a `serve`. Comparing
process identifiers invites the reuse problem recovery already had to solve for runs. A lock on a
file of its own is released by the operating system however the Tower ends, so "is anybody holding
it" is exactly "is that Tower alive", and a record written by a release that kept no owner is
treated as a Tower that has gone.

**Why a restart is charged what the run it replaces spent.** A chain's ledger lives in the Tower's
memory and starts afresh after a restart. Charging the interrupted run's measured spend — and
counting it as a run — before the restart is admitted keeps recovery a rail rather than a way for a
chain to buy itself a new budget by being interrupted.

**Why a released join is restarted past its barrier.** The barrier state went with the Tower that
held it, and decisions above discard it on restart. Delivered to a fresh barrier, the restarted join
would wait for upstreams whose runs finished before the restart and be given up as unreachable. So
the live record keeps the one flight carrying every arrival, marked as already released, and a
restart delivers it straight to its agent.

**Why history files a run by the day it finished.** The run was written by hand into the segment of
the day it *started*, while the history reader looks for a run by its finish. An hour-long run that
crossed midnight landed in a segment its own window never opened, and parallel runs appending by
hand could interleave. Every run now goes through `History::append`, which files by finish and takes
the segment's lock.

**Why shared files are locked per path, in one registry.** Runs of one agent now overlap, and they
share its Hangar's `memory.md`, the factory's learnings and logbook, and the journal's help, reports
and queue. Each read-modify-write takes a lock keyed by the file's path from one process-wide
registry, so two handles on the same journal share it, and each appended line is one write of the
whole line. Across processes nothing is shared, which is why two Towers over one factory remain
unsupported: see risk 23.

**Why `doctor` measures free slots rather than waiting time.** Work waiting in a busy factory is the
design. What the one-at-a-time Tower produced was waiting *with slots free*, so that is what is
measured: from history, how long each run spent queued while fewer than `max_concurrent_runs` runs —
and fewer than its agent's own cap — were alive. Over `timeout_sec` of that is a warning.

**Why spawning needs a generation counter.** A spawned itinerary gets fresh Hops, fresh Fuel and a
fresh run cap — that is the entire point, because per-item work wants per-item budget. It is also
exactly what makes spawning unbounded: Hops counts depth *within* a chain and cannot see across
chains, so an agent that spawns an agent that spawns an agent recurses forever while every
individual chain stays perfectly inside its rails. Generation is Hops one level up, and it is the
only thing standing between a spawn route and a fork bomb that no existing rail can see.

**Why spawning is a route mode rather than a capability.** The first draft had the scanner call
`layover_spawn` with no route between it and the reviewer, and validation immediately reported the
reviewer as unreachable — correctly, because reachability only knows about routes. The deeper
problem was that `layover_send` is checked against the route map and a free-standing spawn would
not have been: an agent able to open a fresh, fully funded chain into any peer is a larger hole
than one able to send that peer a message. `mode = "spawn"` keeps the route map the single source
of truth for who may reach whom, and makes reachability, the hop check and the diagram all work
without special cases. A route may not both spawn and join, because a barrier waits for upstreams
within one itinerary while a spawn opens one per flight, so every spawned chain would arrive alone
and park forever — silently stalled work rather than an error anybody would see.

**Why a Layover is work set down rather than a chain kept alive.** A chain that publishes a pull
request and then wants to answer the comments arriving on it over the following days had no
expressible shape. Polling spends a hop and real money every tick, so Hops kills it long before a
human replies — and Hops is right to. Re-triggering on a bare schedule works mechanically but
arrives knowing nothing: which work item, what was tried, what the earlier chain concluded. So a
run books a layover, and a resuming pipeline opens a *new* itinerary seeded with a `Handover`.
Nothing stays alive in between — no process, no parked chain, no held budget — which is the same
answer recovery and steering reached, for the same reason: what the later run needs is the earlier
one's context, not its process. Checks back off and eventually expire, because something waiting
on a human who has moved on must stop costing money, and the difference between waiting patiently
and leaking is a count.

**Why a resumed run is handed what woke the booking run and what it reported, cut to size.** A
layover's whole case over a bare schedule is that the later run arrives knowing which work this is,
and it was handed one line — what the earlier run said it was waiting for. The two things the Tower
holds without interpreting anything are the message that woke the run that set the work down,
which is where a work item is named, and that run's own `layover_report`, which is where it says
what it concluded; a transcript summary would need a model, and the last lines of output are
usually the middle of a thought. The message is captured when the layover is booked, the report
looked up when it is resumed because a run usually reports after it books. Each is quoted, so it
reads as something an earlier run was told or said rather than as instructions, and cut to 2,000
characters from the start, because both state their point first and the handover joins a payload
that already runs to tens of kilobytes.

**Why a workflow is a view over pipelines rather than a new concept.** A factory with three
pipelines was drawn as one graph, and a reader looking at it reasonably concluded that Layover
models a single, very confused process. The pipelines were always separate workflows — a nightly
sweep has nothing to do with taking a work item to a pull request — but every surface flattened
them: the route map drew every route, cost broke down by agent and by model but never by pipeline,
and `RunCost` did not even carry the pipeline that a `RunRecord` already recorded. Nothing new was
needed in the model; what was missing was that the boundary the model already had was invisible
everywhere it mattered. Scoping is a view concern, so `Scope` lives in the diagram module rather
than becoming a fourth thing to configure. *Later amended:* routes gained a `pipelines` scope (see
below), which is a permission on the route rather than a view setting. The diagram's `Scope` is
still a view concern; it now draws a workflow over the routes its chains may actually use.

**Why routes can be scoped to pipelines, and why that keeps the mesh a mesh.** One mesh for the
whole factory meant every chain could use every route, and a real factory's workflows share agents.
Karl's review sweep spawns a reviewer per pull request; the build workflow lets the same reviewer
hand work to the only agent that pushes code, for its self-review. The sweep's reviewer reads
untrusted pull request text, and the route map permitted it that edge — only a prompt said not to.
A second cost was quieter: the per-workflow map drew agents a workflow never uses, and a pipeline
had to declare every flag of every prompt the shared mesh let it reach. A scope says *who may reach
whom within a workflow*, so every question the mesh answered is still answered by an edge, and
nothing about order is introduced — agents still decide where work goes. The scope sits on the
route, not on the pipeline, so that `[[routes]]` stays the one place that says who may reach whom
and pipelines stay thin: an entry agent, a trigger, some booleans.

**Why an unscoped route stays global.** Every route written before scopes existed has to mean what
it meant, and the common case — a factory with one workflow, or routes every workflow shares —
should not have to name anything. Narrowing is opt-in and additive: a route gains a scope, never
loses one. `pipelines = []` is refused rather than read either way, because "no pipelines" is a
route nothing may use and "global" is the widest permission there is; a typo in either direction
would be silent.

**Why a spawned chain inherits its pipeline.** A spawn exists to give per-item work its own budget
— Hops, Fuel, run cap. It was never meant to change what the work may do. A spawned chain that lost
its pipeline would lose every route scoped to the workflow that spawned it, and one that was
granted a different pipeline would be exactly the widening scopes exist to prevent.

**Why a resumed layover is narrowed rather than simply re-scoped.** A resuming pipeline collects
every layover that comes due, and any agent may book one, so the obvious rule — the resumed chain
uses the resuming pipeline's routes — let a chain reach another workflow's agents by setting its
work down and waiting: a sweep's reviewer, talked into booking a layover by the text it was
reviewing, is woken by the build follow-up with a route to the agent that pushes code, and the
handover even quotes the text that talked it into it. Three alternatives were weighed. Keeping the
booking chain's pipeline would be simplest and safe, but it changes which pipeline follow-up work
is labelled, costed and flagged under — a behaviour change for every factory that resumes, scoped
or not. Letting `resumes` list the pipelines it collects from is explicit, but safe only for an
operator who knows to write it. So the resumed chain belongs to the resuming pipeline as before —
its flags, joins, spawn edges and labelling — and may use an edge only when every pipeline it
descends from also permits it. The ordinary follow-up, resuming its own workflow's work, loses
nothing because both permit the same edges; an unscoped factory is unaffected because every graph
is the same graph; and the ancestry is a set, so a layover set down a hundred times narrows no
further than once. The narrowing is recorded by the Tower on the layover and on queued work, never
taken from an agent.

**What was rejected for scoping.** A route table per pipeline would have been a second route map to
keep agreeing with the first, and a copy of every shared route per workflow. A deny-list (`except =
[...]`) fails open: a pipeline added later would silently gain every route nobody thought to
exclude it from. Scoping by agent groups or tags would have added a fourth concept to configure
for what pipelines already name. And sequencing — "in this workflow, `a` then `b`" — was never on
the table: it is the pipeline engine the route map deliberately is not.

**Why validation takes the best case across a pipeline's ways in.** The reach, hop and join checks
have always proved only the negative: an agent *no* way in can wake, a depth *no* way in can
afford. Scoped, each way in is walked over its own graph and the findings merged by that same best
case, so a factory that scopes nothing gets exactly the findings it always had. The one new check
that looks for something *dead* — a scoped route no chain of its pipelines can wake — treats every
agent any chain can wake as a possible start for a resuming pipeline, because resumed work goes
back to whoever booked it and any agent may book; a warning that is not true is one people stop
reading.

**Why the dashboard still draws one diagram per workflow once routes are scoped.** Scopes invite a
combined map with each edge styled by workflow, and the page does not draw one. Separate diagrams
were chosen because a combined one reads as a single confused process, and scoping makes that
worse, not better: the reader now has to filter edges by colour to see what one workflow can do,
which is the separation the per-workflow diagram does for them. So each workflow is drawn over its
own routes and nothing else. The whole-factory picture still exists for the question it answers —
"what may anything reach, anywhere" — as `layover graph` and `GET /graph` without a pipeline, and
there a scoped edge is labelled with its pipelines, so it is never mistaken for a global one.

**Why workflow membership crosses spawn edges when reachability does not.** The two ask different
questions. `reachable_from` asks what could still deliver into *this* itinerary, and a spawned
chain never can — that is what stops a dead barrier being kept alive by an agent that could not
possibly satisfy it. Workflow membership asks what a way in sets in motion, and a reviewer spawned
by a sweep is unarguably part of the sweep. An agent belonging to two workflows appears in both,
which is the honest answer rather than the tidy one.

**Why a manual trigger queues rather than runs, and says so.** The dashboard was read-only on the
grounds that a control which silently does nothing is worse than no control — it gets trusted once
and then relied upon at the moment it matters. A trigger button that could not dispatch anything
would be exactly that. What makes it honest is that the thing it produces is real: a durable,
persisted flight, which the Tower drains. The window names the Tower that will pick it up; on a
dashboard that runs nothing (`--watch-only`) it says nothing here will, and
`PendingList.dispatched_by` is `null` rather than a plausible name, so the queue never looks like
it is moving when it is not. A Ground Stop refuses the trigger,
because a kill switch that halts running work while letting more be booked is not a kill switch.

**Why a queued trigger is a Flight and not a new noun.** The obvious modelling is a "request" or a
"booking" that later becomes a flight. But a trigger that has not been dispatched *is* a flight
that has not been dispatched, and the vocabulary already asks a reader to hold twenty-five terms —
a count an independent design review called out as the single biggest comprehension cost in the
project. Adding a twenty-sixth to describe a state of an existing one would be paying that cost for
nothing.

**Why an agent writes its own report rather than the Tower keeping the transcript.** A transcript
is not a report: it contains every approach the agent abandoned, so reading one to find out what
happened is slower than doing the work again. The Tower could store transcripts and separately ask
for a summary, but requiring the agent to state its own conclusion has a second effect worth more
than the storage — an agent that must write down what it concluded is an agent that has to decide
what it concluded. Reports are capped and truncated rather than rejected, because a report is the
only account of a run that has already happened and already cost money, and discarding it to punish
a formatting mistake loses the thing entirely. A trimmed report says it was trimmed, so a reader
knows to look further rather than assuming the agent stopped there.

**Why the model an agent runs on is read from its command line.** The route map and `GET /agents`
show each agent's model, reasoning effort and context tier. The obvious source was `agent.model`,
and it was wrong for the factories where the question matters most: those that fix the model in
the runner command, one runner per model and permission set, and so declare no `model` at all.
A new `context` or `effort` field was rejected for the reason `{model}` exists instead of a
`model_flag` field — every CLI spells these differently, and the runner command is already the one
place that knows how to invoke a given CLI; a second place would disagree with it the first time
somebody edited one. Reading the command line with the agent's model substituted is also the only
honest answer: a declared model whose runner has no `{model}` placeholder never reaches the CLI.
Only flags whose meaning is certain are read — `--model`, and Copilot CLI's `--reasoning-effort`
and `--context` — because a display that guessed at unknown flags would state a wrong model with
the same confidence as a right one.

**Why a route each way is drawn as one line.** Most routes in a real factory come in reciprocal
pairs, and the layered layout drew every reverse direction as a return path: a loop under the
whole diagram in a lane of its own. A twenty-route workflow became eleven loops stacked four
hundred pixels deep, crossing everything on the way, and the map stopped answering the question it
exists for. Merging a pair into one two-headed line loses nothing — the line says exactly "either
may send to the other" — so long as only *plain* pairs are merged: a join, a spawn or a differing
scope makes one direction mean something the other does not, and each keeps its own arrow. A route
between two agents in the same column gets a short connector or arc beside the column for the same
reason; only a route that genuinely runs backwards still goes round the bottom. Interaction —
hovering an agent to light its routes — was added too, but not as the fix: it helps someone who
already knows which agent to ask about, and a map has to be readable before anyone touches it.

---

## Still open

Fifty questions were answered on 18 September 2026; the reasoning is in
[`first-release.md`](first-release.md). What follows is what remains genuinely undecided.

Agents should ask rather than guess on any of these.

1. **Idle detection.** Whether "no output for N minutes" should be a rail alongside wall-clock
   `timeout_sec`. It needs per-runner knowledge of what counts as output, and for unattended
   spending the distinction between thinking and wedged is worth real money. Deferred, not
   dismissed.
2. **Two-phase Fuel reservation.** The floor is an approximation; reserving before a run and
   reconciling after is the real answer. Recorded as risk 15.
3. **Dispatch-aware barriers.** Wanted the moment a factory has several rarely-needed expensive
   helpers. Held back by a race that needs a dispatch window.
4. **A code of conduct.** Absent deliberately; the argument for adding it strengthens the moment a
   first outside contributor appears.
5. **Continuity.** A bus factor of one, stated plainly rather than solved.
6. **How a Codex run is told about MCP servers.** `codex exec -c` accepts only a dotted
   `key=value` override — `-c mcp_servers.layover.url="…"` — and has no flag that reads a file, so
   the documented `mcp = { flag = "-c", format = "codex_toml" }` wiring passes a path Codex refuses
   and every Codex run with MCP fails before it starts. Layover writes the right *content*
   (`[mcp_servers.<name>]` tables, with `env_vars` and `bearer_token_env_var` so no secret is in
   the file) but nothing can hand it over. The options: expand the file into one `-c` pair per key
   (no secret needs to reach the command line, since the token and credentials are all named by
   variable); point `CODEX_HOME` at a per-run directory, which would also hide the operator's own
   Codex login; or drop Codex MCP wiring until Codex reads a file. *Recommendation:* the `-c`
   expansion, as a new `format = "codex_overrides"` so the runner still says what it wants, tested
   against a real `codex exec` before it ships.
7. **Credentials for an HTTP MCP server.** `env_from` on a `url` server puts the variable in the
   agent CLI's environment, which a remote server cannot read, and there is no way to say "send
   this variable as the `Authorization` header". The reference factory's `ado` server declares
   `ADO_PAT` and would reach Azure DevOps unauthenticated. Both Claude Code and Copilot CLI expand
   `${NAME}` in `headers`, so a `headers_from = { Authorization = "Bearer ${ADO_PAT}" }` — names
   only, validated like `env` — would keep the secret out of every file. *Recommendation:* add it,
   and until then have `validate` warn that `env_from` on a `url` server does nothing.
8. **What a worktree is, before any is built.** `access = "read-only"` and `workspace =
   "per-itinerary"` are declared everywhere and implemented nowhere: every agent runs in its
   `work_dir`. The decisions above settle *that* they get worktrees; these settle *how*, and each
   changes what the reference factory does:
   - **Which tree a read-only snapshot starts from.** "The current commit" does not contain a
     developer's *uncommitted* change, which is exactly what the tester and reviewer are asked to
     judge. Either the developer must commit before handing off (a prompt rule, fragile), or the
     snapshot is taken from the working tree including uncommitted changes (a copy, not a
     worktree), or read-only agents in a chain share that chain's writer's tree read-only.
     *Recommendation:* in a `per-itinerary` pipeline, read-only agents run in the itinerary's own
     worktree — the tree they are judging — and a separate snapshot is only taken for a
     `read-only` agent in a `shared` pipeline, from `HEAD` plus a note in its payload that
     uncommitted work is not visible.
   - **One per itinerary, or one per run.** Per run is safest and costs a checkout per test run;
     per itinerary is cheaper and lets a loop's reviewer see the tester's build.
     *Recommendation:* per itinerary for `per-itinerary` pipelines, per run for a read-only agent
     in a `shared` one.
   - **Cleanup.** When the itinerary goes quiet — no run live, no barrier parked, nothing queued —
     or only after a retention period, so a stalled chain's tree can be inspected. And what happens
     to a branch the worktree created. *Recommendation:* keep it until the itinerary is quiet and a
     day has passed, prune with Hangars, never delete a branch with unpushed commits.
   - **A `work_dir` that is not a git repository.** Refuse at load for a factory that declares
     either setting, fall back to the shared tree with a warning, or copy. *Recommendation:*
     `validate` errors when `work_dir` is not a repository and isolation is declared, because a
     silent fallback is the state the project is in today.
   - **A resumed layover**, which opens a new itinerary about an old one's pull request: new
     worktree from the pull request's branch, or the old itinerary's kept tree.
9. **Two global routes that disagree about one edge.** Scoped routes that a chain could use
    together must agree about `mode` and `join` for any pair they share, and `validate` refuses
    them when they do not. Two *global* routes naming the same pair, one spawning and one not, were
    never checked: the edge silently spawns. It was left alone so that no existing factory's
    findings changed. *Recommendation:* report it as the error it is at the next major version,
    with the same message the scoped case uses.
10. **Checking a resumed chain against what it can actually reach.** Validation measures a
    resuming pipeline's reach, hop depth and flags from its `entry`, as it always has — but resumed
    work goes back to whichever agent booked the layover, and the chain is narrowed by its booking
    pipeline. So a resumed agent whose prompt tests a flag the resuming pipeline does not declare
    still fails at composition, not at load. *Recommendation:* for each resuming pipeline, check
    the flags of every agent any chain can wake (the same starting set the dead-route check uses);
    it can only add findings, which is why it waits for a major version.

## Beyond the first runnable release

Rough ordering, not commitments.

- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- `join = "any"`, quorum joins, and joins that wait only for the upstreams actually dispatched
- Non-boolean pipeline parameters, if booleans turn out not to be enough
- `include = [...]` for multi-file factory definitions
- Two-phase Fuel reservation, replacing the floor
- Idle detection alongside wall-clock timeouts
- Authenticode and Apple notarization, once the warning costs more than the keys
