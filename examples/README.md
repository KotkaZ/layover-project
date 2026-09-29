# Examples

Five factories, smallest first. Each one loads — the test suite parses and validates all of them
under `--strict`, so none can quietly rot.

| | Factory | Agents | What it introduces |
|---|---|---|---|
| 1 | [`planner.toml`](planner.toml) | 3 | The whole file format in one screen: runners, agents, a pipeline, a route map. Walked through in [Your first factory](https://kotkaz.github.io/layover-project/first-factory.html). |
| 2 | [`news-digest/`](news-digest/) | 2 | A **schedule**, and the read-only/read-write split. The smallest thing worth actually running. |
| 3 | [`pr-review/`](pr-review/) | 2 | **Spawn fan-out** — one itinerary per pull request, each with its own Fuel, over a count nobody knows in advance. |
| 4 | [`build-and-review/`](build-and-review/) | 5 | **Routes scoped to workflows** — two workflows share a reviewer, and only one of them may hand the builder work. |
| 5 | [`workitem-factory/`](workitem-factory/) | 9 | Everything awkward at once: two rendezvous joins, a rework loop of unknown length, three ways in, conditional prompts, per-agent MCP servers. |

Read them in that order. The first four take a few minutes each; the fifth has a
[long walkthrough](workitem-factory/README.md) because the interesting parts are the
arithmetic and the reasoning, not the TOML.

## Trying one

```sh
layover --config examples/news-digest/layover.toml validate --strict
layover --config examples/news-digest/layover.toml explain
layover --config examples/news-digest/layover.toml prompt scout
layover --config examples/news-digest/layover.toml serve
```

Nothing spawns a process yet, so this is the whole loop: check a factory, read the prompts it
would send, and watch the dashboard. See
[Status](https://kotkaz.github.io/layover-project/#status).

## A warning about the fifth

`workitem-factory/` is written against real Azure DevOps and a real Kusto cluster, and its
publisher opens pull requests. It defaults to draft (`draft_pr = true`) and the publisher's prompt
tells it to open nothing rather than guess — but it is an example of a factory that *acts*, not a
sandbox. Read §6 of its README before pointing it at anything you care about.
