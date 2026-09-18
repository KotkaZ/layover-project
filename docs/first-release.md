# Decisions for the first runnable release

Fifty questions were put to the maintainer as a worksheet and answered in one pass on
18 September 2026. This file carries the reasoning behind each answer — *why*, not *what*, in the
same shape as [the decision log](decisions.md).

The split between the two files is what the decisions describe. [`decisions.md`](decisions.md)
explains the system that exists; this explains the one being built. Each entry moves across as the
code that embodies it lands.

Scope and open questions are in [`decisions.md`](decisions.md); known hazards in
[`risks.md`](risks.md); the design itself in [`architecture.md`](architecture.md).

---


Fifty questions were put to the maintainer as a worksheet and answered in one pass on
18 September 2026. What follows is the reasoning behind each answer, in the same shape as the
decision log above: *why*, not *what*. Where an answer was close, the counter-argument is recorded
too, because a decision whose alternative has been forgotten cannot be revisited honestly.

### Starting a run

**Why the flight body goes last in the payload.** A run may be handed five things — instructions,
memory, learnings, a handover, and the message that woke it. Models weight the start and end of a
context differently, and whatever arrives last reads as the current instruction. The body *is* the
instruction; everything above it is context for carrying it out. The handover sits immediately
above the body because "you are continuing work that did not finish" only means something next to
what the work is, and learnings sit above the handover so a recovery instruction is never buried
under twenty-five lines of accumulated advice. Implemented in `payload.rs`, with the order pinned
by a test rather than left to whoever edits next.

**Why memory is injected rather than fetched.** The alternative — an agent calling
`layover_memory_read()` when it wants its own notes — is cheaper and explicit, and it fails
silently: an agent that forgets to call has no memory, and nothing anywhere reports that. Since
fresh runs are what make memory deliberate in the first place, a memory system that quietly does
not work would undo the decision it was built to serve. So the tail is always injected, capped, and
says when it was cut; the whole file remains a tool call away. The tail rather than the head
because the end of the file is the most recent thing written, and a memory that kept only its
oldest entries would get less useful the longer an agent ran.

**Why `model` reaches the CLI through a `{model}` placeholder.** Each supported CLI spells the flag
differently — `--model`, `-m`, a config key. A `model_flag` field on the runner would put half an
invocation in one place and half in another; the placeholder keeps the runner command the single
thing that knows how to invoke a given CLI, and lets the operator write `--model x` or
`--model=x` as theirs requires. The failure it creates — an agent declaring a model whose runner
cannot carry it — is caught by validation rather than discovered from a bill.

**Why the Tower tells an agent its own name.** Three words, and without them every prompt file
hard-codes the name of the agent it belongs to, which drifts the first time one is renamed. It has
to come from the Tower for the same reason everything else about identity does: a name an agent
asserts is a name an agent can get wrong.

**Why output is streamed to disk rather than collected at exit.** One write path, a transcript that
survives the Tower dying mid-run, and a live view for free. The cost is that parsing a runner's
cost reporting becomes incremental rather than a single read, which is fiddlier for `stream-json`
formats than it sounds. Idle detection — distinguishing a run that is thinking from one that is
wedged — is deferred: it needs per-runner knowledge of what counts as output, and wall-clock
`timeout_sec` is a coarse but honest stand-in until then.

### Ending a run

**Why Ground Stop signals before it kills.** Killing immediately corrupts whatever a read-write
agent was half-writing, and an operator burned by that once will hesitate before using the kill
switch again. A kill switch people hesitate over is not a kill switch. So: block new spawns at
once, signal, then kill after a grace period. Parked barriers survive, because a Ground Stop is a
pause — you engage it to look at something — and losing parked work would mean the factory cannot
be safely paused. Tokens are revoked, because otherwise a child mid-call can still send a flight
after the factory has stopped, which would make the stop a lie.

**Why a dead-end run is a success that poisons its barriers.** An agent that exits without sending
anything has not failed; not every agent forwards work, and marking it failed would make the
dashboard cry wolf. But a barrier waiting on that agent can now never be satisfied, and the
itinerary has to learn that immediately rather than waiting forever. Silent permanent stalling is
the worst outcome in the system, so the run is recorded as fine and the barriers it stranded are
marked unsatisfiable at once.

**Why a timeout does not trigger recovery.** Recovery exists for interruptions, where the cause has
gone away — the machine restarted, the Tower died. A timeout means the work was too big or
something is wedged, and both of those repeat identically, so retrying spends money to arrive in
the same place. The run is killed, marked `timed_out`, and the sender is told, so a loop can decide
what to do about it. An agent's `recovery = "automatic"` deliberately does not override this: a
timeout and an interruption are different events and deserve different defaults.

**Why a crash reuses the recovery policy.** A crash and an interruption are the same shape of
problem — a run that did not finish and left no verdict — and `recovery` already exists to say
whether repeating a given agent's work is safe. A second, parallel policy would be two knobs for
one question. The difference is that a crash leaves an exit code, so the handover can carry it:
"the previous run exited with status 137" is a useful thing for its replacement to know.

**Why an itinerary is done when it goes quiet.** Quiescence — no run live, no barrier parked —
needs no cooperation from any agent, which matters because agents are exactly the part that cannot
be relied upon to report. An agent may still end a chain explicitly. This also forces an
itineraries endpoint into existence, which is overdue: `stalled` is an itinerary state with
nowhere to live, and the dashboard cannot currently distinguish a finished chain from a quiet one.

### Concurrency

**Why two simultaneous triggers make two itineraries.** Deduplicating needs a notion of "the same
request" that nothing supplies, and getting it wrong silently drops work somebody asked for. Two
people triggering the same pipeline a second apart probably both meant it.

**Why Hops remains the only bound on a loop.** A per-route loop counter would be a third depth rail
to explain, and the vocabulary is already the largest comprehension cost in the project. The real
fix is arithmetic the factory author can do — the reference factory carries it as `flights = 2N + 6`
— so validation warns when a cycle exists and `max_hops` leaves room for fewer than two rework
rounds. The counter-argument is real and was weighed: exhausting Hops mid-repair leaves
half-finished work in a workspace, and "we warned you at load time" is thin comfort when it
happens.

**Why the Tower enforces barrier reset.** A loop-back must re-dispatch the whole fan-out; if the
developer sends only to the tester on a second pass, the reviewer's slot never fills and the chain
stalls forever. Leaving that to prompt text made correctness depend on an agent remembering an
instruction. The dispatch-wave tracking needed to enforce it already exists — it went in to stop
`join = "any"` firing once per upstream — so the Tower resets a barrier that receives a flight from
a new wave. Detection stays as the safety net: a barrier that becomes unsatisfiable marks the
itinerary stalled immediately.

**Why joins still wait for every declared upstream.** Dispatch-aware barriers have a genuine race —
a fast upstream can release the barrier before its sibling is dispatched at all — and closing it
needs a dispatch window in the most correctness-sensitive part of the system, where a mistake
produces exactly the silent stall everything else here is designed to prevent. The workaround costs
one cheap run per skipped specialist. It stops being a good trade when a factory has several
rarely-needed expensive helpers, and that is the signal to revisit.

**Why a scheduled pipeline skips a tick it is still working on.** Starting a second copy means
paying twice for one result, and possibly two agents writing to one workspace. Skipping means being
an hour late. For unattended spending those are not comparable. `overlap = "allow"` opts in, and a
skip is logged — a schedule quietly skipping every tick because its job always overruns should be
visible.

### Restarts

**Why run state is written before the process starts.** A run that was live when the Tower died
leaves no exit code, so the only way to know it existed is to have said so first. The record says
what should be running and a liveness check confirms it is gone, which feeds the distinction
`Interruption` already makes between "confirmed gone" and "unknown". The PID is stored with the
process start time, because PIDs are reused and recovering into a world where something else now
owns that number is worse than not recovering.

**Why a handover carries memory and sent flights.** Both are facts the Tower holds without
interpreting anything. "You already sent a work item to the developer" is the single most useful
thing a resumed run can know, because it is precisely what it would otherwise duplicate. A
transcript summary would need a model, which Layover does not have; the last N lines of output are
usually the middle of a thought.

**Why a parked barrier is discarded on restart.** Keeping partial state means waiting for upstreams
whose runs no longer exist and will never arrive — a stall dressed as patience. Re-dispatching
repeats work and costs money, but it terminates. The itinerary records that it was recovered, so
the repeated work is explicable rather than mysterious.

**Why a Layover cannot be booked past the retention horizon.** Work that wakes up after its own
history has been pruned wakes into an itinerary it cannot see the past of, which makes its handover
a stub. Defaulting the limit to the horizon keeps "a resumed chain can read its own history" true
by construction.

### Money

**Why an implausible cost is treated as unreported.** Negative, `NaN` and infinite figures are
already ignored and `$0` alongside real tokens is already treated as silence, but a plausible-looking
figure an order of magnitude too low is believed — and Fuel and the Reserve then under-count
together. Marking it `Unreported` reuses machinery that already does the right thing: an unreported
run downgrades the confidence of every total it appears in, so the dashboard says "this is a lower
bound" instead of showing a precise-looking lie. Substituting a rate-card estimate would invent a
different number with no better claim to truth; refusing the runner outright is too blunt for what
might be one malformed line.

**Why spawning gets a count-based cap as well as the Reserve.** The Reserve is the right
factory-wide rail conceptually and it depends on runners telling the truth about cost. A count of
spawned itineraries is something the Tower knows for certain. When the honest rail and the
cooperative rail disagree about how much is happening, the honest one needs to exist too. The cost
is a fifth number bounding breadth, and a scanner that legitimately finds forty pull requests now
reviews only the first N — which is its own kind of quiet wrong, and the reason the default is
generous rather than tight.

**Why Fuel gets a floor rather than a reservation.** Metering after the fact lets an itinerary
overshoot by one full invocation, and with fan-out by several at once. Proper two-phase reservation
— estimate, reserve, reconcile — is the correct answer and makes the ledger substantially more
complex, including a new question about what happens when the estimate was wrong. Refusing to start
a run when remaining Fuel is below a floor is most of the benefit for a fraction of the work: it
stops a chain with two cents left starting a four-dollar run. The real fix stays recorded as a risk
rather than being quietly renamed as done.

**Why a Reserve refusal has to be loud.** "Nothing is running" and "nothing is *allowed* to run"
must not look the same, or an operator spends an afternoon debugging a factory that is working
exactly as configured. So: a banner naming the time the window rolls forward, a warning at 80%, an
automatically raised help request — because that is where an operator already looks for "why is
this stuck" and it puts the event in history — and a cap that can be raised by editing config and
reloading, because hitting a ceiling at 2am should not mean bouncing the Tower and losing running
work.

**Why a dangerous factory is confirmed rather than forbidden.** Every rail is configurable, so
every rail can be configured to be useless. A compiled-in absolute ceiling would be wrong for
somebody and would be worked around by exactly the people it was meant to protect. Showing the
implied worst case in money and runs at the moment of the decision, and requiring an
acknowledgement recorded in state, educates instead of forbidding. The counter-argument was
weighed: for a tool whose failure mode is a five-figure bill, a hard ceiling with an explicit
escape is defensible, and this is the entry to revisit if one ever gets close.

### Trust

**Why credentials are injected per run rather than inherited.** Inheriting the Tower's environment
means the telemetry agent holds the publishing credentials, the reviewer holds the cloud keys, and
a single prompt injection reaches all of them. Injecting exactly what an agent's `env_from` names
is the entire reason that field names variables instead of holding values, and it makes revocation
meaningful: change one variable and the next run of one agent is affected.

**Why the per-run token is the identity.** A token that can be derived from the agent name is not a
token. It is minted per run, random, passed in the environment, and the Tower looks up which agent
and which itinerary it belongs to — so the child never asserts either, which is what §4.2 requires.
It expires with the run, is revoked on Ground Stop, timeout, crash and recovery, and an
unrecognised token is refused *and* raised as an incident rather than dropped: it means a bug or a
child that outlived its run, and both are worth knowing about.

**Why learnings are delimited, filtered and gated by impact.** A learning applies to twenty runs
with no human in the loop, and its text comes from an agent whose own input may include a work
item or a pull request comment. That makes it a more durable foothold than any single prompt
injection. Expiry already bounds the blast radius and an echo cannot confirm a learning; what
remained was that the text itself was trusted. Delimiting it as untrusted costs nothing and helps
most; refusing text that names tools, URLs or credentials or tries to override earlier
instructions costs a few false negatives; confirming high-impact learnings before they apply costs
a human press on the rare case that warrants one.

**Why the prompt sandbox canonicalises.** Lexical confinement handles `..`, absolute paths and UNC,
and does not handle a symlink inside the prompt directory pointing anywhere at all. Today prompt
files are reviewed repository content and anyone who can plant a symlink can also set
`runners.*.command`, so it is not yet a boundary — but it becomes one the moment agents write their
own prompts, which is a stated goal, and by then it is load-bearing. Doing it now is a small change
with no downside; doing it later means doing it under pressure.

**Why the local API is authenticated by default.** Loopback is a real boundary and it stops being a
sufficient one when the thing behind it can spend money — a shared machine, or a web page in a
browser reaching localhost. Minting a token at startup and printing it in the dashboard URL is a
proven pattern for exactly this situation and costs the operator one copy-paste. `--no-auth` exists
for people who genuinely do not want it. The counter-argument: it is friction on the most-used
command of a tool one person runs on their own laptop.

**Why a factory definition is made visible rather than gated.** A `layover.toml` names runner
commands, so running someone else's is arbitrary code execution by design — it is a script that
says which programs to run. A hard `--trust` gate would be annoying for the ninety-nine percent
case of your own config, and annoyance is worked around with a flag people stop reading. So
`explain` shows the runner commands prominently, `validate` warns when a config's hash differs
from the last one seen and names what changed, and the documentation says plainly that a factory
definition is code.

### Platform

**Why Windows is first-class.** It is where the project is developed, and it is where the hard
parts differ — killing a process tree is not sending a signal to a process group. "Best-effort" on
the platform the author uses daily is a promise that quietly becomes false. CI runs there and a
Windows failure blocks a release.

**Why the Logbook stays one file.** It is meant to be read by a human during an incident and edited
by hand when an agent records something wrong; splitting it per agent to avoid a lock trades away
the property that justified the format. Serializing every write through the Tower keeps both, and
append-only removes the mode where two agents rewrite the same file and one silently loses.

**Why structured logs come before traces.** An itinerary is obviously a trace and a run obviously a
span, and fan-out and joins are exactly what trace viewers are good at — but OpenTelemetry is a
dependency, a collector and an operational story, and the thing has to run before it needs
beautiful traces. Emitting `tracing` spans with the itinerary and run as fields from the start
makes adding an OTel layer later a subscriber change rather than an instrumentation rewrite.

### Shape

**Why half the aviation vocabulary stays and half goes.** A design review named the vocabulary the
single biggest comprehension cost for a newcomer, and it was right about the decorative half. The
test applied was whether a term names something plain English has no word for. **Hops**, **Fuel**,
**Ground Stop**, **Layover**, **Itinerary**, **Slots** and **Reserve** pass — "TTL" is wrong for
Hops, "budget" misses that Fuel depletes as you travel, and "causal chain" is both clumsier and
longer than Itinerary. **Tower**, **Hangar** and **Logbook** fail: they are a supervisor, a state
directory and shared memory, and naming them otherwise makes a reader learn a word to understand a
directory listing. Consistency has real value and a half-metaphor can read as indecision, which is
the argument for keeping all of it; what settled it is that this was the last moment the choice was
nearly free.

**Why `entry = true` is removed.** Two ways to say how work gets in is one too many. A pipeline
with no flags expresses what `entry = true` expressed, in the place people already look for ways
in, and removing it also removes the `to` field from `POST /flights`, a branch from flag
resolution, and the question of what happens when a bare entry agent is triggered and its prompt
tests a flag. The counter-argument is that the two mean subtly different things — `entry` is a
permission, a pipeline is a named way in — and collapsing them loses the ability to say "you may
poke this agent, but there is no blessed workflow for it". Nobody had used either, which was the
only evidence available.

**Why a factory stays one file.** One file means the whole factory is readable at once, which is
what makes `explain` and the route map comprehensible. The reference factory is 259 lines and that
is not yet a problem. Adding `include` later is easy and backward-compatible; removing it later is
neither. The risk taken is that if factories routinely reach twenty agents, the pain arrives during
this release's life rather than after it.

**Why the MCP surface is fixed and enforced.** Prompts and documentation named eleven tools, none
of which existed, with names that disagreed between the prompts, the book and the code. The surface
is now settled — send, peers, report, help and memory as the minimum; learn and logbook next;
status and wait if they earn it; spawn never, because `mode = "spawn"` on a route already covers it
and a tool would be a second permission model over the same graph. The part that matters beyond the
list is that the registry lives in code and `validate` rejects a prompt naming a tool that does not
exist, which is exactly the check that would have caught the drift.

**Why the dashboard gets four write controls.** Ground Stop, because a kill switch reachable only
by creating a file by hand is not one anybody reaches for in a hurry. Cancelling a queued flight,
because without it the trigger button is one-way. Confirming and rejecting a learning, because the
documented justification for applying learnings without approval depends on revocation being one
press, and shipping with that still untrue would repeat the exact mistake this project keeps
finding in its own documentation. Resolving a help request, because otherwise the open count only
grows. Steering a running itinerary and retrying a failed run wait.

### Release

**Why the crates are published and marked not-public-API.** `layover-cli` was unclaimed, and the
downside of squatting is somebody shipping malware under a name the documentation tells people to
trust. Publishing claims it. Marking the library crates as not public API in their own
documentation keeps the freedom to restructure internals without a major bump, which matters
enormously while the supervisor is still being written — and it is what makes single Apache-2.0
licensing correct, since the Rust dual-licence convention exists to let GPLv2 projects consume
*libraries*.

**Why provenance and not signing.** A checksum proves a download was not corrupted; it says nothing
about who produced it, and it sits beside the file it describes. Build attestation turns "here is a
hash" into "this was built from that commit by that workflow" and is free. Authenticode and Apple
notarization cost money and key management and buy the removal of a scary dialog — worth it when
there are enough users for the dialog to matter, not before.

**Why the maintainer protects the branch against themselves.** The objection to branch protection
in a single-maintainer repository — there is nobody else to protect against — cuts the other way.
The person most likely to tag a half-finished commit at 2am is the only person who can, and a guard
rail set for yourself is the only kind available. Requiring CI to have passed, restricting who can
create `v*` tags, and requiring a tagged commit to descend from `main` cost nothing during normal
work.

**Why the broken installers are documented rather than patched.** The generated shell installer
skips verification when `sha256sum` is missing — which is stock macOS — and the PowerShell one does
not verify at all. Patching 1,598 lines of generated script creates a maintenance burden that rots
silently the first time upstream restructures it, and a silently rotted checksum patch is worse
than none because the documentation would again claim something untrue. So: report it upstream, say
plainly what is and is not checked, and give a fail-closed manual path.

**Why the on-disk layout is versioned.** Without a version key, two releases silently disagreeing
about the format is a data-loss bug waiting for its moment. A newer layout is refused rather than
read hopefully; an older one is migrated forward once, and says so. There is no self-update: a tool
that rewrites its own binary while potentially supervising running children is a category of bug
nobody needs, and the installers already handle it.

### What the first runnable release has to prove

**Why "it ran for a week" rather than "it worked once".** The reference factory completing end to
end is the criterion the design was sized against, and it is not sufficient. This project claims
*lights-out*, and every interesting failure in the system is a failure of the second day: a stalled
barrier, a schedule overlapping itself, a run interrupted by a laptop sleeping, a Reserve window
rolling over. None of those appear in one successful pass. So the bar is seven days unattended with
the operator only reading the dashboard, and any intervention resets the clock and becomes a bug.
The cost is a slow, unglamorous gate; the alternative is shipping a claim that has been tested once.

**Why a generic runner moves into scope.** "We support these three CLIs" is the most likely reason
somebody forks this instead of using it. A runner defined entirely by its command, its stdin
behaviour and its cost parsing makes a new CLI a config entry rather than a code change, and it is
mostly machinery that has to exist for the three supported ones anyway.

---

