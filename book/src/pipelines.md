# Pipelines and triggers

A route map says which agents *may* talk to each other. A pipeline says how work **gets in**:
which agent receives it, whether a human or a clock starts it, and which flags the run is
parameterised by.

```toml
[pipelines.development]
description = "Take a request through investigation, development and review to a pull request"
entry       = "analyst"
trigger     = "manual"

[pipelines.development.flags]
run_e2e  = { default = false, description = "Also run the remote end-to-end suite" }
draft_pr = { default = true,  description = "Open the pull request as a draft" }

[pipelines.review-bot]
description = "Review my open Azure DevOps pull requests once an hour"
entry       = "pr_scanner"
trigger     = { every = "1h" }
```

Pipelines are deliberately thin. They do not describe a sequence of steps, so adding one does not
turn the permission mesh into a pipeline engine.

## Triggers

| Form | Meaning |
|---|---|
| `trigger = "manual"` | A human starts it. The default. |
| `trigger = { every = "1h" }` | Fires on a fixed interval: `s`, `m`, `h`, `d`. |
| `trigger = { cron = "0 9 * * 1-5" }` | Fires on a five-field cron expression, in local time. |

## Running several instances at once

One pipeline, many instances — one per pull request, say. Each trigger mints its own itinerary
with its own Hops, Fuel, barriers and flags, so instances are already independent in every respect
but one: **the workspace**.

```toml
[pipelines.development]
entry     = "analyst"
workspace = "per-itinerary"
```

| Value | Meaning |
|---|---|
| `shared` | Every itinerary works in the one `work_dir`. The default. |
| `per-itinerary` | Each itinerary gets its own git worktree, named after the itinerary. |

With `shared`, two instances that both reach a `read-write` agent edit the same files at the same
time. That fails in the way hardest to notice — plausible output built from two unrelated changes.
`per-itinerary` is what makes parallel instances safe.

`layover validate` warns when a **scheduled** pipeline uses `shared` and reaches a writer, because
a schedule overlaps itself with nobody watching. It does not warn for manual pipelines: a human
starting a second instance knows they did, and warning on every manual pipeline with a writer
would fire on almost every factory.

Setting both `every` and `cron` is an error rather than a silent choice between them.

### The one-minute floor

A schedule may not fire more often than once a minute. Every firing is a real, paid CLI
invocation, and a schedule runs with nobody watching. Six-field cron expressions — the ones with
a seconds column — are refused for the same reason: a seconds field can schedule work faster than
a run can finish, which is a fork bomb with a clock attached.

### Schedules do not queue

Runs are reentrant. A schedule that fires again before the previous run has finished does not
queue behind it — it starts a second, concurrent copy of the same itinerary. `layover validate`
warns when an interval is shorter than `timeout_sec`:

```text
warning: pipeline `review-bot` fires every 300s but a single run may take 1800s;
         runs are reentrant, so slow runs will overlap rather than queue
```

A cron expression has no single interval to compare against, so that check stays silent rather
than guessing. Sizing a cron schedule is on you.

## Flags

A flag is a boolean a pipeline accepts at trigger time and a prompt can test:

```toml
[pipelines.development.flags]
run_e2e = { default = false, description = "Also run the remote end-to-end suite" }
```

```sh
layover prompt tester --pipeline development --flag run_e2e=true
```

Rules worth knowing:

- **A flag name must be an identifier.** It has to survive being written inside `@include(...)`.
- **Setting an undeclared flag is an error**, not a no-op. A typo at trigger time would otherwise
  change nothing while appearing to work.
- **Two pipelines may declare the same flag, but not with different defaults.** Prompts are shared
  between pipelines, so the same `@include(run_e2e)` line is read by every pipeline that reaches
  that agent. Disagreeing defaults make it mean different things depending on which trigger fired.
  `layover validate` warns.

## Entry points

An agent is an entry point when a pipeline names it, **or** when it is marked `entry = true`.

These are different things. `entry = true` is a bare permission — useful for an agent you want to
poke by hand. A pipeline is a named trigger that also carries a schedule and flags, and it is the
normal way in.

A factory with neither cannot be triggered at all, which is an error.

## Sizing the rails

This is the part most likely to be got wrong, because nothing computes it for you.

A chain carries at most `max_hops` flights: the trigger is flight 1, and each send spends one hop.
For a factory with a loop, count the loop:

```text
flights = lead_in + 2N + 1
```

where `lead_in` is the flights spent before the looping agent's first run and `N` is the number of
times the loop turns. For the reference factory that is `2N + 6`, so eight review cycles need
`max_hops = 22` — against a default of 8.

`layover validate` warns when an agent sits further from an entry point than `max_hops` can reach,
and when a `join = "all"` barrier has an upstream that could never afford the flight into it:

```text
warning: agent `target` waits for every upstream, but `c` could only deliver on flight 4 and
         `max_hops` is 3; the barrier can never release and the itinerary would stall
```

That second check matters because plain reachability misses it. A joined agent looks close if *any*
upstream is close, but it does not wake until the **last** one arrives.

Neither check will catch an undersized *loop* budget, because both measure shortest paths and no
static check can know how many times a loop will turn.

Getting it wrong is not a clean failure. Hops running out mid-repair leaves half-finished work in
the shared workspace and no run alive to clean it up.
