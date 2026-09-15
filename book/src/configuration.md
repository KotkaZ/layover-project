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
| `http_addr` | `127.0.0.1:7878` | Where the API binds. Loopback by default, deliberately. |

## `[defaults]` — the safety rails

| Key | Default | Meaning |
|---|---|---|
| `runner` | — | Runner used by agents that do not name one. |
| `max_hops` | `8` | Maximum flights in one chain. Bounds **depth**. |
| `fuel_usd` | `5.00` | Shared cost budget for an itinerary. Bounds **breadth**. |
| `max_runs` | `64` | Deterministic run cap; holds when a runner reports no cost. |
| `timeout_sec` | `900` | Wall-clock limit for one run. |

`max_hops` and `fuel_usd` are not interchangeable. A hop is spent per flight and branches *inherit*
the remaining count rather than splitting it, so Hops says nothing about how wide a fan-out
spreads. At `max_hops = 8` with a branching factor of 3, one trigger permits roughly 6,500 real,
paid CLI invocations. Fuel is what stops that, and `max_runs` is what stops it when the runner
does not report its cost.

## `[runners.*]` — how to invoke a CLI

```toml
[runners.claude]
command = ["claude", "-p", "{prompt}", "--output-format", "stream-json"]
mcp     = { flag = "--mcp-config", format = "claude_json" }
```

`{prompt}` is substituted at spawn time. `mcp` says how this runner is told where Layover's MCP
server is.

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
| `resident` | `false` | Pin the agent resident rather than transient. Not in v0.1. |
| `fuel_usd` | — | Fuel override for itineraries that *start* at this agent. |

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
| `mode` | `async` only. `request_response` was superseded by joins. |
| `join` | `all` or `any`. Parks flights until the condition is met. |
| `timeout_sec` | Backstop for a barrier that never completes. |

Direction is explicit. An edge absent from `[[routes]]` means the flight is refused.

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
