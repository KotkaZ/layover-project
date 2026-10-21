# Workflows, triggers and spawning

Part of the [decision log](../decisions.md): pipelines, schedules, manual triggers, spawning, layovers, follow-ups and per-workflow rails.

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

**Why a schedule is stored as a person with a note beside it.** The richer `Origin` could not simply
be written as new variants. A 1.0.0 binary reading the queue drops a line it cannot parse and then
rewrites the queue without it, so a downgrade would silently lose scheduled and resumed work. The
on-disk flight keeps the two senders 1.0.0 knows and puts the rest in an optional `via` field it
ignores: a downgrade loses a label, never the work, and the state directory's layout — whose bump
would be a breaking change — stays as it is.

**Why the overlap warning reads a cron expression's minute field.** A schedule that outruns its
own work accumulates concurrent runs rather than queueing, so the check needs a lower bound on how
often a pipeline can fire. `every` states it outright. Cron does not, but the common footguns —
`* * * * *` and `*/5 * * * *` — state it in the minute field, and reading only those two forms is
enough to catch them. Lists and ranges are deliberately left alone: a wrong lower bound produces a
warning that is not true, and a validator that cries wolf is one people stop reading.

**Why parallel instances needed only a workspace change.** "One pipeline instance per pull
request" sounds like it needs an instance concept, and it does not: every trigger already mints
its own itinerary with its own Hops, Fuel, barriers and flags, because barriers are keyed by
`(itinerary_id, to_agent)` and Fuel is per chain. The single thing instances actually shared was
the working directory. `workspace = "per-itinerary"` is to give each a git worktree, and the rest
was already true. Left as opt-in because a worktree per itinerary costs disk and setup time, and a
single-instance pipeline wants neither. *Not built yet*: the setting is accepted and does nothing,
which `layover explain` says; see open question 8.

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
one's context, not its process. Nothing gives up on a layover by itself: a resumed run that finds
nothing books a new one, for a wait it chooses. A back-off and an expiry after twelve checks were
designed for this and written, and never wired to the resume path, so for every release up to
1.7.0 they were documented and did not happen; they were removed rather than left to look like a
bound. When to stop waiting is a judgement about the work — a review that is a day late and one
that is a month late are different — so it lives in the prompt of the agent that waits, and the
Reserve bounds what all the checks cost together.

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

**Why a workflow changes a shared agent rather than declaring its own.** A real factory's reviewer
did the same job in two workflows at different scales — one change at a time in development, a dozen
pull requests every two hours in a sweep — and the only way to run it more cheaply in the sweep was
a second agent, with its own prompt file, notes and routes, or a second runner repeating the whole
deny list. Either copy drifts from the first. `[pipelines.<name>.agents.<agent>]` changes the one
agent in that workflow's chains: `model`, `effort` and `context` replace the agent's own, because a
run has exactly one of each, and `args` are only added after the runner's and the agent's, so a
workflow can tighten an agent and never loosen it. It is a table of its own beside the workflow's
flags rather than among them (decided with the maintainer, 2026-10-08): a flag is a boolean a prompt
tests and a person flips when triggering, while these are a CLI's values, and mixing the two would
turn the flags into a permissions form. Which workflow a chain runs in is the Tower's record, as it
is for scoped routes, so the line an agent runs with is worked out at launch from that record and no
agent can ask for another workflow's. A change for an agent the workflow never reaches is warned
about, because it changes nothing.

**Why a workflow's switch closes its ways in and nothing else.** Asked for as an on-off switch for
every workflow, scheduled ones included, and the only stop there was halted the whole factory and
killed what was running. Two choices were the maintainer's (2026-10-12). Chains of a workflow
already under way **finish**: holding their hand-offs would strand a chain half done, with Fuel
spent and barriers waiting, and ending their runs is what the Ground Stop already does for anybody
who wants that. And the switch is **runtime state, not configuration**: it is flipped while the
factory runs and has to survive the restart that reloads `layover.toml`, and a key in the file
beside it would be a second answer to "is this on?" that the two could disagree about. So it is a
line in the journal, read by the Tower on every pass like the Ground Stop's file, and settable by
hand. What "a way in" is had to be stated rather than read off a flight: every flight of a chain
carries its pipeline, so the Tower holds only a chain's *first* flight — from a person, a schedule
or a resumed layover — and never a hand-off, a spawn, a restart, a released join or the continuation
a reply starts, all of which belong to work already under way. A trigger is refused rather than
queued to wait, because a person told "queued" walks away believing their run is coming; one queued
before the switch was flipped waits rather than being cancelled, because cancelling is a decision
the dashboard already offers and taking it silently would lose work somebody asked for. Ticks that
pass while a workflow is off are neither fired later nor recorded as skips. Catching up on switching
back on would start work at a moment nobody chose, which is what the switch was flipped to prevent;
and a skip is recorded because a skipping schedule looks from outside like one running fine, while a
switched-off workflow says it is off wherever it is shown. Switching on asks for no confirmation,
unlike releasing a Ground Stop, because it starts nothing at once beyond the triggers it held.

**Why a resuming pipeline's `entry` is optional, and `null` in the API.** A resuming pipeline wakes
whoever set its work down, so `entry` was a required key that did nothing — except decide where a
manual trigger of it would go, which is how a follow-up pipeline came to send a hand-triggered run
to an agent with nothing to follow up. Left out, the pipeline cannot be triggered by hand; given,
it keeps that one meaning. The API's `entry` became nullable rather than an empty string, because
an empty agent name is a value a client would try to use. Validation, the route map and `explain`
measure an entry-less pipeline from every agent that sends on its routes, since any of them may
have set the work down.

**Why a follow-up names where its routes come from, rather than routes naming it.** Five of a
production factory's routes carried `pipelines = ["devforge", "devforge-follow-up"]`, and a route
added for DevForge that forgot the second name would quietly leave the follow-up unable to finish
work it resumed. The follow-up's relationship is to a workflow, not to each route, so it is said
once on the pipeline: `routes_from = "devforge"`. It is resolved as the file is read, by adding the
follow-up to each of those routes' scopes, so the route map — what `explain` prints, what the Tower
checks, what the dashboard draws — still says on every route who may use it, and nothing that reads
routes changed. It widens nothing a resumed chain can do beyond what the booking chain could: that
narrowing still applies on top.

**Why a workflow sets its own rails, and an agent's Fuel finally applies.** One set of rails had to
fit every workflow, so a factory sized for a development chain that loops through review for an
afternoon — one in production ran on `max_hops = 256`, `max_runs = 400`, `fuel_usd = 5000` — left a
review spawned per pull request practically unbounded (the maintainer approved per-workflow rails,
2026-10-12, as a change to how rails are set). A workflow may now set `max_hops`, `fuel_usd`,
`max_runs` and `timeout` for its chains; nothing it sets reaches the Reserve, which bounds the
factory. Who wins follows the per-agent precedence elsewhere: the workflow, then — for Fuel — the
agent the chain starts at, then `[defaults]`. Tracing where a chain's budget is minted showed the
agent's own `fuel_usd`, documented since 0.x as the Fuel of a chain starting there and shown on the
dashboard, was never applied: every chain was minted from `[defaults]`. It is applied now, as
documented, rather than the documentation being corrected to match — a per-agent Fuel nobody can set
would be the rail that looks configured and is not. A chain keeps the rails it was minted with, so
editing `layover.toml` never changes the budget of work already under way. A restarted run's chain
is re-minted from its workflow's rails without the agent's Fuel, because which agent it began at is
not on record.
