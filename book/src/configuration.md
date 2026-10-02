# Configuration

One file, `layover.toml`. Unknown fields are **rejected**, not ignored: a typo should fail while
a human is still watching.

## `[layover]` — where things live

| Key | Default | Meaning |
|---|---|---|
| `state_dir` | `.layover/state` | One hangar per agent: memory, transcripts, run history. |
| `work_dir` | `workspace` | The shared working directory agents operate in, relative to this file. |
| `logbook` | `.layover/logbook.md` | Shared memory. All writes serialised by the Tower. |
| `prompt_dir` | `prompts` | What `prompt_file` paths resolve against, relative to this file. |
| `http_addr` | `127.0.0.1:7878` | Where the API binds. Loopback by default, deliberately. **Not yet honoured** — `layover serve --addr` sets the bind address today. |

### The state directory is versioned

`.layover/version.json` records which shape the directory is, and Layover checks it before reading
or writing anything:

```json
{
  "layout": 1,
  "written_by": "0.16.0"
}
```

A directory written by a **newer** release is refused, and the command stops:

```text
error: this state directory is layout 99, written by Layover 9.9.9, and this build understands
       layout 1. Upgrade, or point at a different directory — reading it anyway would drop
       whatever the newer release added.
```

That is deliberate. An older build cannot know what it does not understand, so reading the
directory anyway means writing it back without whatever was added — which turns "I downgraded for
an afternoon" into permanent loss. Refusing is recoverable; the other way is not.

An **older** layout is migrated forward once and says so. A directory with no marker at all — one
from before versioning, or a fresh one — is stamped as current, which is right because versioning
arrived before the shape ever changed.

`written_by` is for a person reading the file. It is never compared against: two builds of one
layout must be interchangeable, or the layout number means nothing.

## `[defaults]` — the safety rails

| Key | Default | Meaning |
|---|---|---|
| `runner` | — | Runner used by agents that do not name one. |
| `max_hops` | `8` | Maximum flights in one chain. Bounds **depth**. |
| `fuel_usd` | `5.00` | Shared cost budget for an itinerary. Bounds **breadth**. |
| `max_runs` | `64` | Deterministic run cap; holds when a runner reports no cost. |
| `timeout_sec` | `900` | Wall-clock limit for one run. |
| `max_recovery_attempts` | `2` | How many times interrupted work may be [restarted](./recovery.md). |
| `max_concurrent_runs` | `4` | How many agent runs may be alive **at once**, factory-wide. Queued work waits for a slot and starts, oldest first, the moment one frees. `1` runs one at a time. |
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

The Tower checks it before every run, against the measured spend in history — dollars a runner
printed, and Copilot credits. At the cap, new runs are refused and recorded as `halted` until
enough spend has rolled out of the window. The default applies to a factory that writes no
`[reserve]` table at all, so every factory has a ceiling unless it says `fuel_usd = 0`.

## `[rates]` — prices, for runners that report tokens but not dollars

```toml
[rates.claude-opus-4]
input_usd       = 5.00
output_usd      = 25.00
cache_read_usd  = 0.50
cache_write_usd = 6.25
```

Optional and always a fallback. Keyed by the model a run's command line selects, and applied only
to a run that printed token counts and no dollar figure at all. Anything derived from it is
labelled an estimate and never folded in as a measurement: it debits no Fuel and draws on no
Reserve — see [Cost](./cost.md#rate-cards) for when it applies and why that distinction is
load-bearing.

## `[copilot]` — what a Copilot AI credit costs

```toml
[copilot]
usd_per_credit = 0.01
```

| Key | Default | Meaning |
|---|---|---|
| `usd_per_credit` | `0.01` | Dollars one Copilot AI credit costs. Must be a positive number; `validate` refuses zero, negative and non-finite values, because zero would record every Copilot run as free *and* measured. |

Copilot CLI reports the AI credits a run used rather than dollars. Layover prices the last
`session.usage_checkpoint` a run prints at this rate, so that Fuel and the Reserve bind a Copilot
factory. The default is GitHub's published price; set it only if you are billed at a different one.
The whole table is optional. See [Cost](./cost.md#copilot-cli-is-priced-from-its-ai-credits).

## `[runners.*]` — how to invoke a CLI

```toml
[runners.claude]
command = ["claude", "-p", "--output-format", "stream-json"]
mcp     = { flag = "--mcp-config", format = "claude_json" }

[runners.copilot]
command = ["copilot", "--allow-all-tools", "--output-format", "json"]
mcp     = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }
```

`mcp` says how this CLI is told where Layover's endpoint is: `flag` is the option, `format` the
dialect of the file written into the run's Hangar, and `prefix` anything that must precede the
path. Copilot CLI needs `prefix = "@"` because `--additional-mcp-config` accepts a JSON string
*or* a path and distinguishes them by that character; most CLIs take a plain path and want no
prefix. See [Agent tools](./tools.md).

### Credentials for the CLI itself

An agent CLI needs a credential before it can do anything, and it is not the same credential its
MCP servers need. Name it in `env_from` — under `[defaults]` when every agent uses the same one,
under an agent when only that agent should hold it:

```toml
[defaults]
env_from = ["GH_TOKEN"]          # every agent's CLI can authenticate

[agents.publisher]
env_from = ["RELEASE_TOKEN"]     # and this one alone can publish
```

Only **names** appear here. The Tower reads each value from its own environment when it spawns the
run, so `layover.toml` stays a file you can commit — putting a secret in it is refused at load
time, not discovered in your git history later.

The two lists are combined, not overridden: the publisher above gets both. A name that is not set
in the Tower's environment **refuses the run**, rather than starting a CLI that fails to
authenticate several seconds later and reports it as the agent's failure.

The child otherwise gets a scrubbed environment — `PATH`, `TEMP`, and the handful of variables a
process needs to start at all. That is what makes `env_from` meaningful: the telemetry agent does
not hold the publishing token because it never receives it.

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

The dashboard, `GET /agents` and the tooltips on the route map report which model each agent
runs on by reading this command line with the agent's `model` filled in. A model you fix in the
command itself — one runner per model is a common shape — is therefore reported as readily as one
an agent declares. Only flags whose meaning is certain are read: `--model` (or `--model=…`), and
Copilot CLI's `--reasoning-effort` and `--context`. Nothing else is guessed at, so a CLI that
spells these differently shows only a model the agent declares.

```toml
[runners.copilot-deep]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh",
           "--context", "long_context", "--output-format", "json"]
```

`mcp` says how this runner is told where Layover's MCP server is.

## `[agents.*]` — who exists

```toml
[agents.tester]
description = "Builds the change and runs the suite, then returns a verdict"
purpose     = """
Route here to find out whether the change works. Builds and runs the suite, and does not edit the
code it is judging. Judges behaviour, never style.
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
| `access` | `read-write` | `read-only` is meant to give the agent a git worktree snapshot. **Declared, not yet enforced** — see below. |
| `entry` | `false` | Whether a human may send flights straight here. |
| `resident` | `false` | Pin the agent resident rather than transient. Not built. |
| `fuel_usd` | — | Fuel override for itineraries that *start* at this agent. |
| `work_dir` | — | Work somewhere other than the shared `work_dir`. Relative to this file; an absolute path is used as written. |
| `recovery` | `automatic` | `manual` if repeating this agent's work would do damage. See [Recovery](./recovery.md). |
| `max_concurrent` | — | At most this many runs of this agent alive at once, within `max_concurrent_runs`. `1` for an agent that must never overlap itself. |

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

**Every declared server reaches the run.** Each run is handed one MCP configuration — the file the
runner's `mcp.flag` points at — and it names Layover's own server under `layover` and every server
the agent declares beside it, in the runner's dialect:

```json
{
  "mcpServers": {
    "kusto":   { "type": "stdio", "command": "agency", "args": ["mcp", "kusto"],
                 "env": { "KUSTO_CLUSTER": "ic3-aria-eus2",
                          "AZURE_CLIENT_SECRET": "${AZURE_CLIENT_SECRET}" } },
    "layover": { "type": "http", "url": "http://127.0.0.1:…/mcp", "headers": { … } }
  }
}
```

The name `layover` is **reserved**: `layover validate` refuses an agent server called that, because
it would replace the one server every run needs to send, report and ask for help.

**No credential value is written into that file.** It lives in the run's Hangar under `.layover/`,
and the whole point of `env_from` is that a secret never sits in a file. The Tower puts each
`env_from` value into the agent CLI's environment and the configuration only *names* it —
`"${NAME}"` for `claude_json`, which Claude Code and Copilot CLI both expand from their own
environment, and `env_vars = ["NAME"]` for `codex_toml`. Copilot CLI additionally passes its whole
environment to the stdio servers it starts; Codex passes only a short allow-list plus `env_vars`,
which is why they are named.

For a `url` server `env_from` only puts the variable in the agent CLI's environment. A remote server
cannot read that, and there is not yet a way to turn it into a request header — see the open
questions in [`decisions.md`](https://github.com/KotkaZ/layover-project/blob/main/docs/decisions.md#still-open).

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

> **Not enforced yet.** `access` is accepted and shown everywhere an agent is described, but
> nothing acts on it: **every agent runs in its `work_dir`**, and a `read-only` agent can write
> there exactly as a `read-write` one can. `layover explain` says so beside the agent list.

What `read-only` is designed to mean is that the agent gets a **git worktree at the current
commit** instead of the live shared workspace, so an inspector cannot disturb work in progress and
is not reading a tree that moves under it. It would not make the filesystem read-only: a tester
could still build and run the suite inside its own checkout. Before that can be built, several
things have to be settled that the design does not yet say — above all, that a snapshot at the
current commit would not contain a developer's *uncommitted* change, which is exactly what the
tester and reviewer are asked to judge. They are recorded as open questions in
[`decisions.md`](https://github.com/KotkaZ/layover-project/blob/main/docs/decisions.md#still-open).

Until then, treat `access` as a statement of intent that prompts should repeat ("do not edit
product code"), not as a guarantee.

Fanning out to two `read-write` agents is a warning: they share one working directory and will
overwrite each other. The same is true of any two agents that run at the same time, whatever their
`access` — and since 1.4.0, runs do.

### Bounding width, not just depth

`max_hops` and `fuel_usd` bound how *deep* and how *expensive* one chain is. Neither bounds how
many agent CLIs are running simultaneously, and that is the number that takes a machine down. A
scanner that dispatches one reviewer per pull request assigned to you produces a fan-out whose
width is not known until it looks.

`max_concurrent_runs` is the rail for it, and it is the only one that **queues rather than
refusing**. Every other rail protects a budget, and money spent is gone. This one protects a
machine, and a machine that is busy now will not be busy in a minute — refusing would turn "review
twelve pull requests" into "review four and silently drop eight".

**Runs overlap.** Up to `max_concurrent_runs` agents run at the same time — the branches of a
fan-out, the reviews a sweep spawns, a schedule's tick beside an hour-long manual job — and the next
queued flight starts the moment a slot frees. Before 1.4.0 the Tower ran one agent at a time
whatever this said, so a factory written then is now more parallel than it has ever been: two
agents that write the same `work_dir` can now write it *at once*. `max_concurrent_runs = 1` restores
one at a time for the whole factory; `max_concurrent` keeps a single agent from overlapping itself:

```toml
[agents.mailman]
max_concurrent = 1   # one Teams sender; every other agent still runs beside it
```

Work starts oldest first among the flights that can start. A flight for an agent at its own cap
waits where it is, and the flights behind it for other agents go ahead: waiting for that agent is
what the cap asks for, and holding the whole factory behind it is not. `layover validate` refuses
either limit at `0`, which would start nothing.

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
| `pipelines` | The pipelines whose chains may use the route. A bare string or a list. Absent means every chain may — see below. |

Direction is explicit. An edge absent from `[[routes]]` means the flight is refused.

### Scoping a route to workflows

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
- The same pair may appear in several routes; the union applies. Two routes one chain could use
  together must agree about `mode` and `join` for any pair they share, and `validate` says so when
  they do not.
- `pipelines = []` and an unknown pipeline name are errors.

A spawned chain keeps its pipeline, a resumed layover belongs to the resuming pipeline but may use
only what the chain that booked it could, and a flight sent straight to an `entry = true` agent
belongs to no pipeline and may use global routes only. The rules, and why, are in
[`routing.md`](https://github.com/KotkaZ/layover-project/blob/main/docs/routing.md#8-scoping-routes-to-workflows).

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

The spawned chain keeps the pipeline of the chain that spawned it, and with it that pipeline's
scoped routes: a spawn gives a chain a fresh budget, not a fresh set of permissions.

### Rendezvous joins

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

## Checking it

```sh
layover validate --strict
```

Errors block startup. Warnings describe shapes that are legal and known to misbehave — an agent
nothing routes to, a fan-out to two writers, a schedule faster than its own runs.
