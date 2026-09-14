# Routing

How flights move between agents: the route map, rendezvous joins, and failure paths.

> Layover is a **permission mesh**, not a pipeline engine. The route map says who *may* send to
> whom. It does not say what happens next — agents decide that. Everything below preserves that
> property.

---

## 1. Edges

```toml
[[routes]]
from = "planner"
to   = "coder"
mode = "async"
```

Direction is explicit: `planner → coder` does not imply the reverse. An edge absent from
`[[routes]]` means the flight is refused. Validation runs at config load, so unknown agent names
and unreachable entry points fail while a human is still watching.

## 2. Fan-out

An agent may send to several peers in one run. They spawn concurrently:

```toml
[[routes]]
from = "planner"
to   = ["probe_a", "probe_b"]
```

Nothing about this is special — it is two ordinary flights.

**Hops does not restrain a fan-out.** A flight costs one hop, and branches inherit the remaining
count rather than splitting it, so Hops bounds *depth* only. With `max_hops = 8` and a branching
factor of 3, one trigger permits up to 3⁸ ≈ 6,500 runs. **Fuel is what bounds breadth**, which is
why it is a v0.1 requirement rather than a later refinement — see
[`roadmap.md`](roadmap.md#why-fuel-is-not-optional).

## 3. Rendezvous joins

Some agents should not wake until several inputs have arrived.

```toml
[[routes]]
from        = ["probe_a", "probe_b"]
to          = "collector"
join        = "all"        # all | any
timeout_sec = 1800
```

The Tower holds a **barrier** keyed by `(itinerary_id, to_agent)`. Flights arriving for a joined
agent are parked rather than delivered. Once the condition is met the target spawns **once**,
receiving all parked flights as its input.

### Why this is still a mesh

The join is a property of the *receiving node*, not a workflow definition. No agent is told what
to do next, and no sequence is enforced. The collector simply cannot be woken by one input alone.
Senders stay free; receivers declare what they need in order to be useful.

This matters because without a join, two edges into one agent make that agent fire **twice** —
and the first firing happens when the *fastest* branch finishes, not when the work is done.
Duplicate side effects are the usual result.

### Abandoning a barrier

A parked barrier that can never complete would hang an itinerary forever, holding work that no
one will collect. Hops does not help, because nothing is flying.

The Tower resolves this by **reachability** rather than by timeout alone: it knows every live run
in the itinerary and the route graph, so it can determine whether any live run could still reach
the barrier. If none can, the barrier is abandoned immediately and the itinerary is marked
*stalled*.

`timeout_sec` remains a backstop for a run that is alive but has wandered off.

**Stalled must be a distinct, visible outcome from failed.** A lights-out factory that quietly
parks work forever is worse than one that crashes.

### Barrier reset

**Rule: a barrier resets when any upstream delivers a second time.** Partial state is discarded,
and every upstream must deliver again.

This exists because of the failure loops in §4. If an upstream re-runs and produces a new result,
a stale result from a *sibling* branch must not be allowed to satisfy the barrier — the collector
would otherwise act on a mixture of old and new state, which looks like success and is not.

The rule carries a constraint: **a loop-back must re-dispatch the whole fan-out, not just the
branch that failed.** Re-sending to only one upstream leaves the barrier waiting for a sibling
that never comes, until reachability analysis abandons it. This currently relies on agent prompts,
which is fragile — see [`risks.md`](risks.md).

## 4. Failure paths

Failure routing is an ordinary edge. Nothing special is required from the Tower, which is what
keeps the mesh intact:

```toml
[[routes]]
from = "probe_a"
to   = "planner"      # something went wrong, hand it back
mode = "async"
```

The sending agent decides which edge to take based on its own result. **Layover does not evaluate
conditions — agents do.** The route map only guarantees that both destinations are permitted.

Because runs are fresh, the flight body must carry enough context for the receiving agent to act.
The agent that wakes is not the agent that did the earlier work and remembers nothing of it. This
is the cost of explicit memory, and prompts must account for it.

## 5. Workspace access

```toml
[agents.probe_b]
access = "read-only"     # read-only | read-write (default)
```

Read-only agents receive a **git worktree at the current commit** rather than the live shared
workspace. This is real enforcement rather than an advisory flag, and it solves a second problem
at the same time: an agent reading the tree while a sibling writes to it would otherwise be
working against a moving target.

This makes the common fan-out shape safe — several agents inspecting concurrently while at most
one writes. **Two concurrent read-write agents remain unsafe**; see [`risks.md`](risks.md).

## 6. Putting it together

A minimal shape exercising every mechanism above: one agent fans out to two, one of which is
read-only, and their results rendezvous at a third.

```
        planner
           │
     ┌─────┴─────┐
     ▼           ▼
  probe_a     probe_b        (concurrent; probe_b is read-only)
     │           │
     └─────┬─────┘
           ▼
     join = "all"
           ▼
       collector
```

```toml
[[routes]]
from = "planner"
to   = ["probe_a", "probe_b"]

[[routes]]
from        = ["probe_a", "probe_b"]
to          = "collector"
join        = "all"
timeout_sec = 1800

[[routes]]
from = "probe_a"
to   = "planner"          # failure loop-back
```

Note that the loop-back edge and the join edge coexist. `probe_a` chooses between them at
runtime, and the Tower's reachability analysis is what keeps the barrier from leaking when it
chooses the loop-back.
