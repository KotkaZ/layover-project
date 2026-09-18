# Configuration

One file, `layover.toml`. Unknown fields are **rejected**, not ignored: a typo should fail while
a human is still watching.

## `[layover]` — where things live

| Key | Default | Meaning |
|---|---|---|
| `state_dir` | `.layover/state` | One hangar per agent: memory, transcripts, run history. |
| `work_dir` | `workspace` | The shared working directory agents operate in. |
| `logbook` | `.layover/logbook.md` | Shared memory. All writes serialised by the Tower. |
| `prompt_dir` | `prompts` | What `prompt_file` paths resolve against, relative to this file. |
| `http_addr` | `127.0.0.1:7878` | Where the API binds. Loopback by default, deliberately. **Not yet honoured** — `layover serve --addr` sets the bind address today. |

## `[defaults]` — the safety rails

| Key | Default | Meaning |
|---|---|---|
| `runner` | — | Runner used by agents that do not name one. |
| `max_hops` | `8` | Maximum flights in one chain. Bounds **depth**. |
| `fuel_usd` | `5.00` | Shared cost budget for an itinerary. Bounds **breadth**. |
| `max_runs` | `64` | Deterministic run cap; holds when a runner reports no cost. |
| `timeout_sec` | `900` | Wall-clock limit for one run. |
| `max_recovery_attempts` | `2` | How many times interrupted work may be [restarted](./recovery.md). |
| `max_concurrent_runs` | `4` | How many agent CLIs may be alive **at once**, factory-wide. Excess work queues. |
| `max_spawn_generations` | `1` | How many `mode = "spawn"` hops separate a chain from the trigger that began it. |

`max_hops` and `fuel_usd` are not interchangeable. A hop is spent per flight and branches *inherit*
the remaining count rather than splitting it, so Hops says nothing about how wide a fan-out
spreads. At `max_hops = 8` with a branching factor of 3, one trigger permits **3,279** real, paid
CLI invocations. Fuel is what stops that, and `max_runs` is what stops it when the runner
does not report its cost.

Nor does Fuel bound the *factory* — it resets with every new itinerary. See `[reserve]` below.

## `[reserve]` — what the whole factory may spend

```toml
[reserve]
fuel_usd     = 120.00   # at most this much...
window_hours = 24       # ...in any rolling 24 hours
```

| Key | Default | Meaning |
|---|---|---|
| `fuel_usd` | `100.00` | Ceiling for the window. `0` means unlimited; anything else must be a positive number, and a negative or `nan` value is refused rather than quietly disabling the cap. |
| `window_hours` | `24` | How far back the rolling window reaches. |

A scheduled pipeline mints a fresh itinerary — and a fresh Fuel budget — on every tick, so an
hourly pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day with every chain inside its rail.
The Reserve is the only thing that sees that. It rolls rather than resetting at midnight, because
a daily bucket can be spent twice across the boundary and needs a timezone to decide where the
boundary is. See [Cost](./cost.md).

## `[rates]` — prices, for runners that report tokens but not dollars

```toml
[rates.claude-opus-4]
input_usd       = 5.00
output_usd      = 25.00
cache_read_usd  = 0.50
cache_write_usd = 6.25
```

Optional and always a fallback. Anything derived from it is labelled an estimate and never folded
in as a measurement — see [Cost](./cost.md) for why that distinction is load-bearing.

## `[runners.*]` — how to invoke a CLI

```toml
[runners.claude]
command = ["claude", "-p", "--output-format", "stream-json"]
mcp     = { flag = "--mcp-config", format = "claude_json" }
```

The prompt goes to the process's **stdin**, never onto its command line, and this is not a style
preference. Windows caps a command line at 32,767 characters. Real agent prompts go well past it:
in a sibling project the review agent's prompt tree composes to roughly 98 KB and its ordinary
developer agent to 34 KB. Inlining the prompt passes every test written against a small fixture
and then fails on the first agent worth running.

`{prompt}` is therefore a **path**, not the text — the file the Tower writes the composed
instructions to before spawning. Include it only for CLIs that accept a file of instructions as a
flag; runners without it have the instructions prepended to the stdin payload instead.

`{model}` carries the agent's `model` into the command. Every CLI spells the flag differently, so
the spelling stays here rather than in a field of its own — write `"--model", "{model}"` or
`"--model={model}"`, whichever yours wants. An agent that declares a `model` whose runner has no
placeholder gets a warning at load time, because otherwise the run would quietly use the CLI's own
default and nothing would say the declaration was ignored.

A bare `"{model}"` argument disappears when no model is set, rather than becoming an empty
argument — several CLIs read an empty string in `argv` as a positional.

`mcp` says how this runner is told where Layover's MCP server is.

## `[agents.*]` — who exists

```toml
[agents.tester]
description = "Builds the change and runs the suite, then returns a verdict"
purpose     = """
Route here to find out whether the change works. Gets its own worktree, so it may build and run
freely. Judges behaviour, never style.
"""
runner      = "codex"
model       = "o4-mini"
access      = "read-only"
prompt_file = "tester.md"
```

| Key | Required | Meaning |
|---|---|---|
| `description` | recommended | One line. Handed to peers by `layover_peers()`. |
| `purpose` | optional | Longer: when to route work here. |
| `runner` | if no default | Which runner invokes it. |
| `model` | optional | Model identifier passed to the runner. |
| `prompt` | one of | Instructions, written inline. |
| `prompt_file` | one of | Instructions from a file, which may [compose others](./prompts.md). |
| `access` | `read-write` | `read-only` agents get a git worktree snapshot, not the live tree. |
| `entry` | `false` | Whether a human may send flights straight here. |
| `resident` | `false` | Pin the agent resident rather than transient. Not built. |
| `fuel_usd` | — | Fuel override for itineraries that *start* at this agent. |
| `work_dir` | — | Work somewhere other than the shared `work_dir`. |
| `recovery` | `automatic` | `manual` if repeating this agent's work would do damage. See [Recovery](./recovery.md). |

### MCP servers

Layover is itself an MCP server — that is how agents send flights. `[agents.<name>.mcp.<server>]`
declares the *other* servers an agent needs:

```toml
[agents.kusto.mcp.kusto]
command  = ["agency", "mcp", "kusto"]
env      = { KUSTO_CLUSTER = "ic3-aria-eus2" }
env_from = ["AZURE_CLIENT_SECRET"]

[agents.publisher.mcp.ado]
url      = "https://dev.azure.com/mcp/"
env_from = ["ADO_PAT"]
```

Give exactly one of `command` (stdio) or `url` (HTTP).

**`env` is for values that are safe in a committed file** — a cluster name, a region. Anything
that authenticates goes in `env_from`, which names variables the Tower forwards from *its own*
environment at spawn time, so the value never appears in `layover.toml`.

`layover validate` **refuses** a literal whose name looks like a credential:

```text
error: agent `kusto` MCP server `kusto` sets `AZURE_CLIENT_SECRET` literally in `env`, and that
       name looks like a credential; move it to `env_from = ["AZURE_CLIENT_SECRET"]`
```

It also warns about plain HTTP to a non-local address, since anything forwarded through `env_from`
would cross the network in the clear.

Exactly one of `prompt` and `prompt_file` must be given. Setting both is an error, because which
one applies would otherwise be undefined.

**`description` is not decoration.** An agent discovering its peers at runtime sees these strings
and nothing else. `layover validate` warns when one is missing.

### Workspace access

`read-only` means the agent gets a **git worktree at the current commit** instead of the live
shared workspace. That is real enforcement, not an advisory flag, and it solves two problems at
once: the inspector cannot disturb work in progress, and it is not reading a tree that moves
under it.

It does **not** mean the filesystem is read-only. A read-only tester can build, run the suite and
write whatever it likes inside its own checkout.

Fanning out to two `read-write` agents is a warning: they share one working directory and will
overwrite each other.

### Bounding width, not just depth

`max_hops` and `fuel_usd` bound how *deep* and how *expensive* one chain is. Neither bounds how
many agent CLIs are running simultaneously, and that is the number that takes a machine down. A
scanner that dispatches one reviewer per pull request assigned to you produces a fan-out whose
width is not known until it looks.

`max_concurrent_runs` is the rail for it, and it is the only one that **queues rather than
refusing**. Every other rail protects a budget, and money spent is gone. This one protects a
machine, and a machine that is busy now will not be busy in a minute — refusing would turn "review
twelve pull requests" into "review four and silently drop eight".

`max_spawn_generations` bounds the other direction. A spawned itinerary gets *fresh* Hops, so Hops
cannot see across chains: without a generation limit an agent that spawns an agent that spawns an
agent recurses forever while every individual chain stays perfectly inside its rails. It is Hops,
one level up.

## `[pipelines.*]` — how work gets in

See [Pipelines and triggers](./pipelines.md).

## `[[routes]]` — who may talk to whom

```toml
[[routes]]
from = "analyst"
to   = ["investigator", "kusto"]      # fan-out: two concurrent runs

[[routes]]
from        = ["investigator", "kusto"]
to          = "analyst"               # fan-in: one barrier
join        = "all"
timeout_sec = 3600
```

| Key | Meaning |
|---|---|
| `from` | Sending agents. A bare string or a list. |
| `to` | Receiving agents. A bare string or a list. |
| `mode` | `async` (default) or `spawn`, which opens a fresh itinerary per flight. `request_response` was superseded by joins. |
| `join` | `all` or `any`. Parks flights until the condition is met. |
| `timeout_sec` | Backstop for a barrier that never completes. |

Direction is explicit. An edge absent from `[[routes]]` means the flight is refused.

### What a join does while the factory runs

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

### When a rendezvous is given up

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

### Spawning

`mode = "spawn"` makes an edge open a **new itinerary** per flight instead of continuing the
current one. The receiver gets its own Fuel, its own hop budget and its own workspace.

That is what makes per-item work affordable. An ordinary `async` edge puts every receiver on one
Fuel budget, so a sweep over twelve pull requests stops partway and *which* ones got done is
whichever finished first.

It is a route rather than a free-standing capability because the route map is the single source of
truth for who may reach whom — a spawn outside it would be an unchecked edge into a fresh, fully
funded chain. A route may not both spawn and join: a barrier waits for upstreams within one
itinerary, so each spawned chain would arrive alone and park forever. Validation rejects it.

### Rendezvous joins

A join is a property of the **receiving** node. It says *which inputs this agent needs together* —
not *when this agent is allowed to run*. A flight from any sender the barrier does not name
bypasses it entirely and wakes the agent on its own, which is what lets a joined agent also be an
entry point.

Two rules fall out of failure handling:

1. **A barrier resets when any upstream delivers a second time.** Otherwise a verdict about the
   *previous* version of the code could satisfy the barrier alongside a fresh one.
2. **Therefore a loop-back must re-dispatch the whole fan-out**, not only the branch that failed.
   Re-sending to one upstream leaves the barrier waiting for a sibling that was never asked.

`join = "all"` waits for **every declared** upstream. There is no such thing as an optional one,
so an agent that consults a specialist only sometimes must dispatch it anyway and let it reply
"nothing to add". See [Prompts](./prompts.md) for the usual way to make that cheap.

## Checking it

```sh
layover validate --strict
```

Errors block startup. Warnings describe shapes that are legal and known to misbehave — an agent
nothing routes to, a fan-out to two writers, a schedule faster than its own runs.
