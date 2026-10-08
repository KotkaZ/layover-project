# Examples

Six factories, smallest first. Each one loads — the test suite parses and validates all of them
under `--strict`, so none can quietly rot.

| | Factory | Agents | What it introduces |
|---|---|---|---|
| 1 | [`planner.toml`](planner.toml) | 3 | The whole file format in one screen: runners, agents, a pipeline, a route map. Walked through in [Your first factory](https://kotkaz.github.io/layover-project/first-factory.html). |
| 2 | [`news-digest/`](news-digest/) | 2 | A **schedule**, and the read-only/read-write split. The smallest thing worth actually running. |
| 3 | [`pr-review/`](pr-review/) | 2 | **Spawn fan-out** — one itinerary per pull request, each with its own Fuel, over a count nobody knows in advance. |
| 4 | [`build-and-review/`](build-and-review/) | 5 | **Routes scoped to workflows** — two workflows share a reviewer, and only one of them may hand the builder work. |
| 5 | [`multi-workflow/`](multi-workflow/) | 8 | **Several workflows on shared agents**, the way a real team's factory grows: one runner for every agent, a shared agent run differently in one workflow, flags declared wherever they are reached, a follow-up that resumes set-down work, and named runs. |
| 6 | [`workitem-factory/`](workitem-factory/) | 9 | Everything awkward at once: two rendezvous joins, a rework loop of unknown length, three ways in, conditional prompts, per-agent MCP servers. |

Read them in that order. The first five take a few minutes each; the sixth has a
[long walkthrough](workitem-factory/README.md) because the interesting parts are the
arithmetic and the reasoning, not the TOML.

## Trying one

```sh
layover --config examples/news-digest/layover.toml validate --strict
layover --config examples/news-digest/layover.toml explain
layover --config examples/news-digest/layover.toml prompt scout
layover --config examples/news-digest/layover.toml serve
```

`serve` runs the factory — it fires the schedule, spawns the agents and serves the dashboard at
the address it prints. Point a factory's `work_dir` at a checkout you are happy for agents to
change, and check the runner commands match the CLIs you have signed in. See
[Status](https://kotkaz.github.io/layover-project/#status).

## A warning about the last two

`workitem-factory/` is written against real Azure DevOps and a real Kusto cluster, and its
publisher opens pull requests. It defaults to draft (`draft_pr = true`) and the publisher's prompt
tells it to open nothing rather than guess — but it is an example of a factory that *acts*, not a
sandbox. Read §6 of its README before pointing it at anything you care about.

`multi-workflow/` acts too: its publisher and messenger speak in a code host and a team chat
through MCP servers that stand in for your own. Its schedules fire with every flag off, so no pull
request is opened unless a person triggers `development` and says so — but the review sweep posts
its reviews whatever the flags say.
