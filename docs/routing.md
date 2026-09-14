# Routing

How flights move between agents: the route map, rendezvous joins, and failure paths.

> Layover is a **permission mesh**, not a pipeline engine. The route map says who *may* send to
> whom. It does not say what happens next — agents decide that. Everything below preserves that
> property.

---

## 1. Edges

```toml
[[routes]]
from = "analyst"
to   = "developer"
mode = "async"
```

Direction is explicit: `analyst → developer` does not imply the reverse. An edge absent from
`[[routes]]` means the flight is refused. Validation runs at config load, so unknown agent names
and unreachable entry points fail while a human is still watching.

## 2. Rendezvous joins

Some agents should not wake until several inputs have arrived. A reviewer's verdict and a test
result are both needed before anything can be announced.

```toml
[[routes]]
from        = ["testing", "review"]
to          = "announcer"
join        = "all"        # all | any
timeout_sec = 1800
```

The Tower holds a **barrier** keyed by `(itinerary_id, to_agent)`. Flights arriving for a joined
agent are parked, not delivered. When the condition is met, the target spawns **once**, receiving
all parked flights as its input.

### Why this is still a mesh

The join is a property of the *receiving node*, not a workflow definition. No agent is told what
to do next and no sequence is enforced. The announcer simply cannot be woken by one input alone.
Senders remain free; receivers declare what they need in order to be useful.

This matters. Without a join, wiring `testing → announcer` and `review → announcer` makes the
announcer fire **twice** — two pull requests — and the first firing happens when the *fastest*
branch finishes, not when the work is actually done.

### Abandoning a barrier

A parked barrier that can never complete would hang an itinerary forever. The Tower resolves this
by **reachability**, not just by timeout: it knows every live run in the itinerary and the route
graph, so it can determine whether any live run could still reach the barrier. If none can, the
barrier is abandoned immediately and the itinerary is marked *stalled* in the UI.

`timeout_sec` remains a backstop for the case where a run is alive but has simply wandered off.

Stalled is a distinct outcome from failed, and must be visible. A lights-out factory that quietly
parks work forever is worse than one that crashes.

### Barrier reset

**Rule: a barrier resets when any upstream delivers a second time.** Partial state is discarded
and every upstream must deliver again.

This exists because of the loop-back path in §3. After the developer fixes a failing test, a stale
review verdict from *before* the fix must not be allowed to satisfy the barrier — otherwise the
announcer opens a PR for code that was reviewed in a different state.

The rule carries a constraint: **a loop-back must re-dispatch the whole fan-out, not just the
failing branch.** If the developer re-sends only to testing, the barrier waits for a review that
never comes, and reachability analysis will eventually abandon it. This is a sharp edge and
should be called out in the agent's prompt.

## 3. Failure paths

Failure routing is an ordinary edge. Nothing special is needed from the Tower, which keeps the
mesh intact:

```toml
[[routes]]
from = "testing"
to   = "developer"      # tests failed, go fix it
mode = "async"
```

The testing agent decides which edge to take based on its result. Layover does not evaluate
conditions — agents do. The route map only guarantees that both destinations are permitted.

Because runs are fresh, the flight body must carry enough context for the receiving agent to act:
which tests failed and where. The developer that wakes is *not* the developer that wrote the code
and remembers nothing. This is the cost of explicit memory, and prompts must account for it.

## 4. Workspace access

```toml
[agents.review]
access = "read-only"     # read-only | read-write (default)
```

Read-only agents are given a **git worktree at the current commit** rather than the live shared
workspace. This is real enforcement rather than an advisory flag, and it solves a second problem
at the same time: a reviewer reading a tree while the testing agent runs a build would otherwise
be reviewing a moving target.

In the pipeline below this is sufficient, because only one of the two parallel agents writes.
Two concurrent read-write agents remain unsafe — see [`risks.md`](risks.md).

## 5. Worked example: a development pipeline

The scenario this design was validated against.

```
   HTTP POST /flights
          │
          ▼
      analyst
          │
          ▼
     developer ──────┬──────────────┐
          ▲          ▼              ▼
          │       testing         review          (parallel)
          │          │              │
          │          └──────┬───────┘
          │                 ▼
          │          join = "all"
          │                 ▼
          │            announcer ──▶ opens a PR
          │                 
          └──── tests failed, fix and re-dispatch
```

```toml
[agents.analyst]
entry  = true
runner = "claude"
prompt = "Turn the incoming goal into a concrete, testable specification."

[agents.developer]
runner = "copilot"
access = "read-write"
prompt = """
Implement the specification in the incoming flight, then commit.
Dispatch to BOTH testing and review — always both, even when re-dispatching after a failure.
"""

[agents.testing]
runner = "codex"
access = "read-write"          # builds and runs tests
prompt = """
Run the test suite.
On pass, send the result to announcer. On failure, send the failing tests to developer.
"""

[agents.review]
runner = "claude"
access = "read-only"           # gets a worktree snapshot
prompt = "Review the change and send a verdict to announcer."

[agents.announcer]
runner = "copilot"
access = "read-write"
prompt = "You receive a test result and a review verdict. Open a pull request."

[[routes]]
from = "analyst"
to   = "developer"

[[routes]]
from = "developer"
to   = ["testing", "review"]   # fan-out

[[routes]]
from = "testing"
to   = "developer"             # failure loop-back

[[routes]]
from        = ["testing", "review"]
to          = "announcer"
join        = "all"
timeout_sec = 1800
```

### What this example still does not answer

- **Which branch does the announcer open a PR from?** Branch-per-itinerary was declined, so no
  component currently creates or names a branch. Unresolved — see
  [`roadmap.md`](roadmap.md#open-questions).
- **How does the announcer get GitHub credentials?** Credential delivery to child CLIs is now
  blocking for v0.1 rather than merely open.
- **How many loop-backs before giving up?** Hops bounds it, but exhausting Hops mid-repair leaves
  a half-finished change in the workspace.
