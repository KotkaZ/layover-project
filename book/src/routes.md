# Routes and joins

The route map says who may send work to whom. Its keys are listed under
[`[[routes]]`](./configuration.md#routes--who-may-talk-to-whom); this page is what scopes,
joins and spawning do while the factory runs. The semantics in full are in
[Routing](./design/routing.md).

## Scoping a route to workflows

A factory with several pipelines is several workflows, and they usually share agents. Without a
scope every chain may use every route, so a review sweep can reach anything the build workflow
can — held back only by what its prompts say, while it reads untrusted pull request text.

```toml
# DevForge may hand bob work from eagle; Eagle Eye spawns an eagle per pull request and may not.
[[routes]]
from      = "eagle"
to        = ["bob", "sherlock"]
pipelines = ["devforge", "devforge-follow-up"]

[[routes]]
from      = "azurix"
to        = "eagle"
mode      = "spawn"
pipelines = "eagle-eye"

[[routes]]
from      = "eagle"
to        = ["azurix", "sherlock"]
pipelines = "eagle-eye"
```

- **Absent** `pipelines` makes a route **global**: every chain may use it, exactly as before scopes
  existed. A factory that scopes nothing behaves and validates exactly as it did.
- **Scoped**, only chains belonging to one of the named pipelines may use it. A chain started by
  `eagle-eye` that asks to send `eagle -> bob` is refused like any edge the map does not draw, and
  `layover_peers` does not list it.
- A pipeline with **`routes_from = "devforge"`** may use every route scoped to `devforge` too, so
  a follow-up need not be named on each of them: `pipelines = "devforge"` above would then serve
  `devforge-follow-up` as well. See [Pipelines](./pipelines.md#which-routes-a-pipelines-chains-may-use).
- The same pair may appear in several routes; the union applies. Two routes one chain could use
  together must agree about `mode` and `join` for any pair they share, and `validate` says so when
  they do not.
- `pipelines = []` and an unknown pipeline name are errors.

A spawned chain keeps its pipeline, a resumed layover belongs to the resuming pipeline but may use
only what the chain that booked it could, and a flight sent straight to an `entry = true` agent
belongs to no pipeline and may use global routes only. The rules, and why, are in
[`routing.md`](./design/routing.md#8-scoping-routes-to-workflows).

## What a join does while the factory runs

A barrier holds **flights**, not processes. The obvious implementation — start the joined agent and
let it block until the rest arrive — costs a live agent CLI per waiting branch, each with a context
window and, under some pricing, a meter running. Parking the flight costs a map entry, and makes
the wait durable: a parked flight is data, a blocked process is not.

| What arrives | What happens |
|---|---|
| The first declared upstream | Parked. `layover run` says who it is still waiting for. |
| The last declared upstream | The agent wakes **once**, with every parked flight, each body labelled with who sent it. |
| A second delivery from an upstream that already reported | A new wave. Partial state is discarded and every upstream must deliver again. |
| An upstream after an `any` join has fired | Dropped, and reported as superseded. |
| Anyone the join does not name — including a human | Straight through. The barrier is untouched. |

The agent wakes **once** because two edges into one agent without a join fire it twice, and for a
publisher that is two pull requests for one piece of work.

A new wave on a second delivery is what makes the develop → test → review loop correct. The
reviewer's approval of the *previous* revision must not combine with a fresh test result for the
one after it, so the moment the tester reports again, the reviewer has to look again too.

## When a rendezvous is given up

A barrier waiting for an upstream nothing can still produce would hold that work forever. Silent
permanent stalling is the worst outcome in this system — worse than a failure, which at least says
something happened — so when a drain goes quiet with a barrier still holding flights, it is
abandoned and named:

```text
Gave up on 1 rendezvous:
  `publisher` will never wake: nothing live can still deliver reviewer (1 flight(s) stranded)
```

`layover validate` catches the version of this that is visible before anything runs — a `join =
"all"` upstream that `max_hops` could never afford the flight into.

## Spawning

`mode = "spawn"` makes an edge open a **new itinerary** per flight instead of continuing the
current one. The receiver gets its own Fuel, its own hop budget and its own workspace.

That is what makes per-item work affordable. An ordinary `async` edge puts every receiver on one
Fuel budget, so a sweep over twelve pull requests stops partway and *which* ones got done is
whichever finished first.

It is a route rather than a free-standing capability because the route map is the single source of
truth for who may reach whom — a spawn outside it would be an unchecked edge into a fresh, fully
funded chain. A route may not both spawn and join: a barrier waits for upstreams within one
itinerary, so each spawned chain would arrive alone and park forever. Validation rejects it.

The spawned chain keeps the pipeline of the chain that spawned it, and with it that pipeline's
scoped routes: a spawn gives a chain a fresh budget, not a fresh set of permissions.

## Rendezvous joins

A join is a property of the **receiving** node. It says *which inputs this agent needs together* —
not *when this agent is allowed to run*. A flight from any sender the barrier does not name
bypasses it entirely and wakes the agent on its own, which is what lets a joined agent also be an
entry point.

A scoped join applies only in its scope: a chain in another pipeline that may reach the same agent
goes straight through.

Two rules fall out of failure handling:

1. **A barrier resets when any upstream delivers a second time.** Otherwise a verdict about the
   *previous* version of the code could satisfy the barrier alongside a fresh one.
2. **Therefore a loop-back must re-dispatch the whole fan-out**, not only the branch that failed.
   Re-sending to one upstream leaves the barrier waiting for a sibling that was never asked.

`join = "all"` waits for **every declared** upstream. There is no such thing as an optional one,
so an agent that consults a specialist only sometimes must dispatch it anyway and let it reply
"nothing to add". See [Prompts](./prompts.md) for the usual way to make that cheap.
