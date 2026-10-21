# Routes, joins and reach

Part of the [decision log](../decisions.md): joins and barriers, Hops at load time, scoped routes and what validation checks.

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

**Why routes can be scoped to pipelines, and why that keeps the mesh a mesh.** One mesh for the
whole factory meant every chain could use every route, and a real factory's workflows share agents.
A production factory's review sweep spawns a reviewer per pull request; the build workflow lets the
same reviewer hand work to the only agent that pushes code, for its self-review. The sweep's
reviewer reads untrusted pull request text, and the route map permitted it that edge — only a prompt
said not to. A second cost was quieter: the per-workflow map drew agents a workflow never uses, and
a pipeline had to declare every flag of every prompt the shared mesh let it reach. A scope says *who
may reach whom within a workflow*, so every question the mesh answered is still answered by an edge,
and nothing about order is introduced — agents still decide where work goes. The scope sits on the
route, not on the pipeline, so that `[[routes]]` stays the one place that says who may reach whom
and pipelines stay thin: an entry agent, a trigger, some booleans.

**Why an unscoped route stays global.** Every route written before scopes existed has to mean what
it meant, and the common case — a factory with one workflow, or routes every workflow shares —
should not have to name anything. Narrowing is opt-in and additive: a route gains a scope, never
loses one. `pipelines = []` is refused rather than read either way, because "no pipelines" is a
route nothing may use and "global" is the widest permission there is; a typo in either direction
would be silent.

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

**Why workflow membership crosses spawn edges when reachability does not.** The two ask different
questions. `reachable_from` asks what could still deliver into *this* itinerary, and a spawned
chain never can — that is what stops a dead barrier being kept alive by an agent that could not
possibly satisfy it. Workflow membership asks what a way in sets in motion, and a reviewer spawned
by a sweep is unarguably part of the sweep. An agent belonging to two workflows appears in both,
which is the honest answer rather than the tidy one.
