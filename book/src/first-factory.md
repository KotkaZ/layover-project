# Your first factory

Three agents, one loop, one way in. This is the whole of `examples/planner.toml`, and it is parsed
and validated by the test suite, so it cannot quietly stop working.

```toml
{{#include ../../examples/planner.toml}}
```

## What each part does

**`[layover]`** says where things live. `prompt_dir` is resolved relative to the configuration
file, so a factory can be run from anywhere.

**`[defaults]`** sets the safety rails. `max_hops` bounds how *deep* a chain of flights can go;
`fuel_usd` and `max_runs` bound how *wide* it can spread. They are not interchangeable — see
[Pipelines and triggers](./pipelines.md#sizing-the-rails).

**`[runners.*]`** says how to invoke each CLI. The composed instructions reach the process on
**stdin**, not on the command line — see [Configuration](./configuration.md#runners--how-to-invoke-a-cli). A `{prompt}`
placeholder, where a runner needs one, is a *path* to that text rather than the text itself.

**`[agents.*]`** declares an agent. The table key is its name. `description` is what peers see
when they ask Layover who they can reach, so write it for another agent to read.

**`[pipelines.*]`** is how work gets in. This one is `manual`: a human starts it.

**`[[routes]]`** is the route map. `planner → coder` does *not* imply `coder → planner`; both
directions are written out. An edge that is not listed means the flight is refused.

## Try it

```sh
layover validate --config examples/planner.toml --strict
layover explain --config examples/planner.toml
layover prompt planner --config examples/planner.toml
layover serve --config examples/planner.toml     # runs the factory and its dashboard; open the address it prints
layover run --config examples/planner.toml --dry-run   # what is queued, without starting it
```

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
