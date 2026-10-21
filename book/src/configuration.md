# Configuration

One file, `layover.toml`. Unknown fields are **rejected**, not ignored: a typo should fail while
a human is still watching.

**Lengths of time** are written the way a schedule is: a number and a unit — `30s`, `15m`, `4h`,
`2d`. Every key that holds one also answers to an older name that carries its unit, and takes a
bare number in it: `timeout = "4h"` and `timeout_sec = 14400` are the same setting, and naming both
is an error. A key in hours or days takes only a whole number of them — `window = "90m"` is refused
rather than rounded.

| Write | Or, as before |
|---|---|
| `[defaults] timeout = "2h"` | `timeout_sec = 7200` |
| `[reserve] window = "24h"` | `window_hours = 24` |
| `[learnings] keep_lapsed = "14d"` | `keep_lapsed_days = 14` |
| `[[routes]] timeout = "30m"` | `timeout_sec = 1800` |

## `[layover]` — where things live

| Key | Default | Meaning |
|---|---|---|
| `state_dir` | — | **Not honoured**, and `validate` warns when it is set. Everything Layover keeps — history, the journal, Hangars, live-run records — is in `.layover/` beside this file. |
| `work_dir` | `workspace` | The shared working directory agents operate in, relative to this file. |
| `logbook` | `.layover/logbook.md` | Shared memory. All writes serialised by the Tower. |
| `prompt_dir` | `prompts` | What `prompt_file` paths resolve against, relative to this file. |
| `http_addr` | `127.0.0.1:7878` | Where `layover serve` binds the dashboard and API. Loopback by default, deliberately. `serve --addr` overrides it for one start, and [`autostart`](./cli.md#autostart) registers `serve` without one, so this is the address a factory started with the computer uses. |

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
| `effort` | — | Reasoning effort for agents that set none; an agent's own wins. Reaches only runners with an `{effort}` placeholder. |
| `context` | — | Context-window tier for agents that set none; an agent's own wins. Reaches only runners with a `{context}` placeholder. |
| `env_from` | `[]` | Names of environment variables forwarded to every agent's CLI: the credential it authenticates with. An agent's own `env_from` adds to this rather than replacing it. See [credentials for the CLI itself](./runners.md#credentials-for-the-cli-itself). |
| `max_hops` | `8` | Maximum flights in one chain. Bounds **depth**. |
| `fuel_usd` | `5.00` | Shared cost budget for an itinerary. Bounds **breadth**. |
| `max_runs` | `64` | Deterministic run cap; holds when a runner reports no cost. |
| `timeout` (`timeout_sec`) | `"15m"` (`900`) | Wall-clock limit for one run. |
| `max_recovery_attempts` | `2` | How many times work may be [restarted](./recovery.md) — after the Tower went away, or after the [model service failed](./recovery.md#when-the-model-service-fails) — counted per piece of work. `0` turns automatic restarts off. |
| `max_concurrent_runs` | `4` | How many agent runs may be alive **at once**, factory-wide. Queued work waits for a slot and starts, oldest first, the moment one frees. `1` runs one at a time. |
| `max_spawn_generations` | `1` | How many `mode = "spawn"` hops separate a chain from the trigger that began it. |

`max_hops` and `fuel_usd` are not interchangeable. A hop is spent per flight and branches *inherit*
the remaining count rather than splitting it, so Hops says nothing about how wide a fan-out
spreads. At `max_hops = 8` with a branching factor of 3, one trigger permits **3,279** real, paid
CLI invocations. Fuel is what stops that, and `max_runs` is what stops it when the runner
does not report its cost.

Nor does Fuel bound the *factory* — it resets with every new itinerary. See `[reserve]` below.

These are the factory's. A workflow may give its own chains different `max_hops`, `fuel_usd`,
`max_runs` and `timeout` in `[pipelines.<name>]`, and an agent's own `fuel_usd` is the Fuel of a
chain that starts at it — see [a workflow's own rails](./pipelines.md#a-workflows-own-rails).

## `[reserve]` — what the whole factory may spend

```toml
[reserve]
fuel_usd     = 120.00   # at most this much...
window_hours = 24       # ...in any rolling 24 hours
```

| Key | Default | Meaning |
|---|---|---|
| `fuel_usd` | `100.00` | Ceiling for the window. `0` means unlimited; anything else must be a positive number, and a negative or `nan` value is refused rather than quietly disabling the cap. |
| `window` (`window_hours`) | `"24h"` (`24`) | How far back the rolling window reaches, in whole hours. |

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

| Key | Default | Meaning |
|---|---|---|
| `input_usd` | `0` | Dollars per million prompt tokens. |
| `output_usd` | `0` | Dollars per million generated tokens. |
| `cache_read_usd` | `0` | Dollars per million tokens served from the prompt cache. |
| `cache_write_usd` | `0` | Dollars per million tokens written to the prompt cache. |

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

## `[learnings]` — how learnings live

```toml
[learnings]
provisional_runs = 20   # runs of its agent a new learning applies for before it lapses
confirm_after    = 3    # rediscoveries that make one permanent
max_chars        = 400  # the longest an agent may propose
max_per_run      = 25   # the most one run is given
keep_lapsed_days = 14   # how long a lapsed one is kept before it is deleted
```

| Key | Default | Meaning |
|---|---|---|
| `provisional_runs` | `20` | Runs of its agent a [learning](./learning.md) applies for before it lapses — counted from when it is proposed, and again from each rediscovery. Runs rather than days, so an hourly agent and a weekly one each get twenty looks at it. |
| `confirm_after` | `3` | Rediscoveries — proposals of a learning while it is lapsed — that make it permanent. An echo of one still being given to runs never counts. |
| `max_chars` | `400` | The longest a learning may be, in characters. A longer proposal is refused, and the agent told the limit. Every learning is in every run of its agent, so this is a recurring cost. |
| `max_per_run` | `25` | The most learnings one run is given: confirmed first, then the oldest. The rest still apply, to the runs after one lapses. |
| `keep_lapsed` (`keep_lapsed_days`) | `"14d"` (`14`) | How long a lapsed learning is kept before it is deleted, in whole days. `0` deletes it as soon as it lapses. |

Each is a trade-off. A longer `provisional_runs` lets a learning help for longer, and lets a wrong
one do damage for longer; a lower `confirm_after` makes true insights permanent sooner, and wrong
ones too — see [risk 19](./design/risks.md). A
change applies to learnings from then on: one already provisional keeps the runs it was given.

Kept, a lapsed learning is what lets a later proposal of the same insight count as a rediscovery.
Deleted, that proposal starts a new learning. So a longer `keep_lapsed_days` gives a true insight
more chance to be confirmed, and a shorter one keeps the list to what is in use: a factory lapsing
nine a day keeps about 125 at fourteen days and none at zero. A lapsed learning goes at the first
run of any agent after its days are up, or when `layover serve` next starts. Confirmed and
provisional learnings are in use and never deleted; a rejected one is kept, because it is what
stops an agent proposing again what a person refused.

`validate` refuses `0` for the first four, which would stop learnings working without anything
failing. What is not configurable: how two learnings are judged to say the same thing, and what a
learning may not say — see [learnings](./learning.md). The whole table is optional.

## `[update]` — hearing about a newer Layover

```toml
[update]
mode               = "notify"   # off | notify | stage | auto
channel            = "stable"   # stable | prerelease
check_every        = "24h"
max_drain          = "4h"       # how long an install waits for what is running
keep_previous      = "7d"       # how long the binary an update replaced is kept for a rollback
# auto only:
window             = "Sat 02:00-06:00"   # local time; leave it out to install when idle
idle_for           = "10m"      # with no window: how long nothing ran or waited
min_age            = { patch = "24h", minor = "48h" }   # how old a release must be
allow              = "minor"    # patch | minor — a major release always waits for a person
require_provenance = true       # install only what `gh` verified
```

| Key | Default | Meaning |
|---|---|---|
| `mode` | `"notify"` | `notify`: the Tower looks for a newer release once per `check_every`, and the [dashboard](./dashboard.md#a-newer-layover), [`doctor`](./cli.md#doctor) and [`layover update check`](./cli.md#update) say what it found and what it changes. Nothing is downloaded and nothing changes. `stage`: as `notify`, and it also gets the release ready to install — see [staging](./updating.md#staging-a-release). `auto`: as `stage`, and it also [installs it unattended](./updating.md#installing-unattended), within the limits below. `off`: the Tower never looks; a person can still run `layover update check`. |
| `channel` | `"stable"` | `prerelease` considers pre-releases too, such as `1.15.0-rc.1`, for trying one before stable factories do. |
| `check_every` | `"24h"` | How often the Tower looks, give or take a twelfth so machines that started together do not ask in the same minute. `validate` refuses less than an hour. A look that fails is retried after an hour, then two, four and so on up to this. |
| `max_drain` | `"4h"` | How long [installing a release](./updating.md#installing-a-release) waits for what is running to finish before cutting it off. `validate` warns when it is shorter than a run's `timeout`, because then the deadline can cut a run off. |
| `keep_previous` | `"7d"` | How long the binary an update replaced is kept beside the new one, for [`layover update rollback`](./cli.md#update). Removed at a start after that. |
| `window` | none | Under `auto`, when an install may begin, in the machine's local time: `"Sat 02:00-06:00"`, `"Mon-Fri 22:00-06:00"`, `"Sat,Sun 01:00-05:00"`, or `"02:00-04:00"` for every day. One that ends before it starts runs past midnight. Without one, an install begins when the factory is idle. |
| `idle_for` | `"10m"` | Under `auto` with no `window`, how long nothing must have run, started, been parked or been queued. |
| `min_age` | `{ patch = "24h", minor = "48h" }` | Under `auto`, how old a release must be — counted from when it was published — before it is installed: the time in which a bad release can be withdrawn. |
| `allow` | `"minor"` | Under `auto`, the largest move: `"patch"` holds a minor release for a person too. A major release always waits for a person. |
| `require_provenance` | `true` | Under `auto`, install only what `gh` verified Layover's release workflow built. `validate` warns when it is `false`. |

What each setting does in practice — how a release is found, staged, installed and rolled
back, and the environment variables that override all of it on a machine — is in
[Updating Layover](./updating.md). The whole table is optional.

## `[runners.*]` — how to invoke a CLI

A runner is **a CLI and a permission set**, shared by every agent with those permissions. For a CLI
Layover knows, name it with `cli`, and say only what this runner takes away:

```toml
[runners.analysis]
cli  = "copilot"
args = ["--disable-builtin-mcps", "--deny-tool=shell(git push)", "--deny-tool=shell(gh:*)",
        "--deny-url=api.github.com"]

[runners.claude]
cli = "claude"
```

| Key | Meaning |
|---|---|
| `cli` | `copilot` or `claude`. Layover supplies the program and everything an unattended run of it needs; see below. |
| `args` | Arguments after what the preset supplies: the runner's own permission set. Only with `cli`. |
| `command` | The whole command, written out, instead of `cli`. For a CLI with no preset (Codex, a script), or a runner that wants none of a preset's defaults. |
| `mcp` | How the CLI is told where Layover's MCP server is. A preset supplies it; given here, it replaces the preset's. |
| `provider_errors` | How a failed run's output tells the model service's failure from the run's own. Copilot CLI is read without it; see [Runners](./runners.md#telling-the-model-services-failure-from-the-runs). |

Exactly one of `cli` and `command` is given; `validate` refuses a runner with neither, or with both.

What each preset supplies, the output a cost is read from, how a failed run is told from the
service's failure, credentials, the CLI's own home and placeholders are on their own page:
[Runners](./runners.md).

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
| `model` | optional | Model identifier, passed to the runner's `{model}`. |
| `effort` | optional | Reasoning effort, passed to the runner's `{effort}`: `high`, `xhigh`, whatever the CLI accepts for the model. Falls back to `[defaults] effort`. |
| `context` | optional | Context-window tier, passed to the runner's `{context}`: Copilot CLI's `default` or `long_context`. Falls back to `[defaults] context`. |
| `args` | — | Arguments added to its runner's command for this agent alone. See [an agent's own arguments](./runners.md#an-agents-own-arguments). |
| `prompt` | one of | Instructions, written inline. |
| `prompt_file` | one of | Instructions from a file, which may [compose others](./prompts.md). |
| `access` | `read-write` | `read-only` is meant to give the agent a git worktree snapshot. **Declared, not yet enforced** — see below. |
| `entry` | `false` | Whether a human may send flights straight here. |
| `resident` | `false` | Pin the agent resident rather than transient. Not built. |
| `fuel_usd` | — | Fuel for itineraries that *start* at this agent: a trigger or tick entering here, or a chain spawned here. A workflow's own `fuel_usd` wins over it. |
| `env_from` | — | Names of environment variables forwarded to this agent's CLI on top of `[defaults] env_from`: a credential only this agent should hold. A name that is not set refuses the run. See [credentials for the CLI itself](./runners.md#credentials-for-the-cli-itself). |
| `mcp` | — | The other MCP servers this agent may reach, keyed by name. See [MCP servers](#mcp-servers). |
| `work_dir` | — | Work somewhere other than the shared `work_dir`. Relative to this file; an absolute path is used as written. |
| `recovery` | `automatic` | `manual` if repeating this agent's work would do damage. Applies to a run interrupted by the Tower going away and to one the [model service ended](./recovery.md#when-the-model-service-fails). See [Recovery](./recovery.md). |
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
questions in [`decisions.md`](./design/decisions.md#still-open).

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
[`decisions.md`](./design/decisions.md#still-open).

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

A pipeline's `description` is one line saying what it is for. `layover explain` prints it under the
pipeline, the route map shows it when the pipeline is hovered, and `GET /pipelines` returns it.

## `[flags]` — flags every workflow accepts

```toml
[flags]
run_e2e = { default = false, description = "Also run the remote end-to-end suite" }
```

| Key | Default | Meaning |
|---|---|---|
| `default` | `false` | The flag's value when whoever triggers the work does not set it, and always on a scheduled tick. |
| `description` | — | What turning the flag on does. `layover explain` prints it beside the flag, and `GET /pipelines` returns it with the flag. |

The same shape as a pipeline's `[pipelines.<name>.flags]`, declared once for every workflow and for
a flight sent straight to an `entry = true` agent. Optional. See
[Flags](./pipelines.md#flags).

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
| `timeout` (`timeout_sec`) | Backstop for a barrier that never completes: `"1h"`, or seconds. |
| `pipelines` | The pipelines whose chains may use the route. A bare string or a list. Absent means every chain may — see below. |

Direction is explicit. An edge absent from `[[routes]]` means the flight is refused.

The order routes are listed in decides how the route map is drawn, and nothing else: the map is
laid out along routes in the order they are declared, so listing them roughly in the order work
flows keeps it readable. See [the route map](./dashboard-chains.md#the-route-map).

Scoping a route to workflows, what a join does while the factory runs, when a rendezvous is
given up, and spawning are on their own page: [Routes and joins](./routes.md).

## Checking it

```sh
layover validate --strict
```

Errors block startup. Warnings describe shapes that are legal and known to misbehave — an agent
nothing routes to, a fan-out to two writers, a schedule faster than its own runs.
