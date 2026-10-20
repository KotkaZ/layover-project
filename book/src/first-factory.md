# Your first factory

Three agents, one loop, one way in, all on GitHub Copilot CLI — so trying it needs that one CLI,
[installed](./install.md#agent-clis) and signed in, and nothing else. This is the whole of
`examples/planner.toml`, and it is parsed and validated by the test suite, so it cannot quietly
stop working.

```toml
{{#include ../../examples/planner.toml}}
```

## What each part does

**`[layover]`** says where things live. `work_dir` is where the agents work, and like every path in
the file it is resolved relative to the configuration file, so a factory can be run from anywhere.
The Tower creates it.

**`[defaults]`** sets the safety rails. `max_hops` bounds how *deep* a chain of flights can go;
`fuel_usd` and `max_runs` bound how *wide* it can spread. They are not interchangeable — see
[Pipelines and triggers](./pipelines.md#sizing-the-rails). `env_from`, commented out here, names
what each run is given from the Tower's environment — before a factory runs for real, a
[CLI home of its own](./runners.md#a-cli-home-of-the-factorys-own), so its sessions stay out
of yours.

**`[runners.*]`** says how to invoke a CLI. For one Layover knows — Copilot CLI or Claude Code —
`cli` is the whole runner, and Layover supplies everything an unattended run of it needs; `args`
lists what its agents are denied. Any other CLI, Codex among them, writes its `command` out — see
[Runners](./runners.md). The composed instructions reach the process on **stdin**, not on the
command line.

**`[agents.*]`** declares an agent. The table key is its name. `description` is what peers see
when they ask Layover who they can reach, so write it for another agent to read. An agent with no
`runner` uses the default one; with no `model` or `effort`, the CLI's own.

**`[pipelines.*]`** is how work gets in. This one is `manual`: a human starts it.

**`[[routes]]`** is the route map. `planner → coder` does *not* imply `coder → planner`; both
directions are written out. An edge that is not listed means the flight is refused.

## Try it

Copy the file into an empty directory as `layover.toml` — agents change what is in their
`work_dir`, so give them a directory of their own — and from there:

```sh
layover validate --strict    # check it before anything runs
layover explain              # what can trigger what, and who may talk to whom
layover prompt planner       # what the planner would actually be told
layover serve                # run the factory; open the dashboard address it prints
```

On the dashboard, start the `build` workflow and give it a goal. `layover run --dry-run` shows what
is queued without starting it.

> **`layover serve` runs the factory.** Trigger a workflow from the dashboard and the Tower
> authorises the flight against the route map and the rails, spawns the agent, watches it, and
> records what happened. When the agent hands work on with `layover_send`, the next agent runs in
> the same chain, on the same budget. `layover run` does the same once, for whatever is queued, and
> then exits. See [Status](./index.md#status).

## What it does not say

Notice what is missing: any statement of what happens *after* the coder finishes. The route map
says the coder *may* send to the reviewer, not that it will. Agents decide that at runtime.

This is the central design choice. Layover is a **permission mesh**, not a pipeline engine. It is
what makes the [reference factory](./reference-factory.md)'s review loop possible without
Layover knowing anything about reviews.
