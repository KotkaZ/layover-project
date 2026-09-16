# Layover

**A local-first framework for running a lights-out agent factory.**

> **Status: early implementation.** The domain core, the generated HTTP surface and the CLI are
> built and tested; nothing spawns a process yet. Documentation:
> [kotkaz.github.io/layover-project](https://kotkaz.github.io/layover-project/) · design:
> [`docs/architecture.md`](docs/architecture.md).

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs, gives them a way
to talk to one another, persists what they learn, and stops them from running away.

You describe a factory in a single `layover.toml` — which agents exist, what each one is for,
which agents may trigger which others, and how work gets in. Layover then runs it unattended.

## Install

Layover is a single binary. No Rust toolchain needed.

```sh
# macOS / Linux
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-installer.sh | sh

# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-installer.ps1 | iex"

# ...or, since you probably already have Node for the agent CLIs
npm i -g https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-npm-package.tar.gz

# ...or with Cargo, if you have it
cargo install layover-cli
```

Other options — direct download, building from source — are in
[the install guide](https://kotkaz.github.io/layover-project/install.html).

```sh
layover validate --config layover.toml --strict   # check a factory before it runs
layover explain                                   # what can trigger what, and what talks to what
layover prompt tester --flag run_e2e=true         # what an agent would actually be told
```

## The idea

Picture an airport grid.

Each **agent** is an airport. A message is a **Flight**. A chain of flights originating from one
trigger is an **Itinerary**, and it carries the two things that keep the network sane: **Hops**
(how many legs remain) and **Fuel** (how much budget remains). The **Tower** is air traffic
control. When everything needs to stop, you call a **Ground Stop**.

## How it works

```
  you ──POST /flights──▶  Tower  ──spawns──▶  claude -p / copilot / codex exec
                            ▲                          │
                            │                          │ MCP: layover_send(...)
                            └──────────────────────────┘
                                   routes, meters, persists
```

- **Sending a message is what starts an agent.** There is no separate spawn step — a flight's
  arrival is what brings an agent to life.
- **Agents talk over MCP.** Layover exposes an MCP server, so Claude Code, Copilot CLI and Codex
  CLI reach it natively with no adapter code.
- **Every run is a clean slate.** Nothing carries over implicitly between runs.
- **Which makes memory explicit.** An agent's continuity is exactly what it chose to write down.
  This is the most opinionated idea in the project, and it is deliberate.
- **The route map is a directed graph.** No edge means the flight is refused.
- **Work enters through pipelines.** Named, triggerable entry points — manual, or on a clock.
- **Prompts compose.** An agent's instructions can pull in extra sections depending on the flags a
  run was triggered with.
- **Runaway swarms are bounded by construction** — every itinerary burns Hops and Fuel, and the
  Tower, not the agent, holds the counters.

## Scope

v0.1 targets Claude Code, GitHub Copilot CLI and OpenAI Codex CLI, on a single machine.
See [`docs/roadmap.md`](docs/roadmap.md) for what is in and out.

## The reference factory

[`examples/workitem-factory/`](examples/workitem-factory/README.md) is the scenario v0.1 is sized
against, and the best place to start reading:

```
human → analyst ⇄ [investigator, kusto]  →  developer ⇄ [tester, reviewer]  →  publisher
```

A request is investigated and backed with telemetry, turned into a work item, implemented, then
tested and reviewed in a loop that turns until both agents approve — after which a pull request is
opened. A second, scheduled pipeline reviews open pull requests hourly. It exercises concurrent
fan-out, two rendezvous joins, a loop of unknown length, conditional prompts, and the hop
arithmetic that makes the loop survivable.

## Building

```
cargo xtask verify
```

That is the only definition of done: generated-code freshness, documentation checks, format, lint,
test and doc build, with warnings denied. CI runs the same command, unchanged.

| Crate | What it is |
|---|---|
| `layover-core` | Config, agents, routes, pipelines, prompts, the route graph, validation, itinerary accounting, barriers |
| `layover-http` | The HTTP surface, **generated** from [`api/openapi.yaml`](api/openapi.yaml) |
| `layover-cli` | The `layover` binary |

Process supervision, the MCP server and the UI are not built yet; they are blocked on open
questions in [`docs/roadmap.md`](docs/roadmap.md).

## This repository is itself agentic-first

Layover is not only *for* agent factories — this repo is built to be worked on by agents.

The governing principle: **verification, not generation, is the bottleneck.** A lights-out
factory does not stall because an agent cannot write code; it stalls because an agent cannot tell
whether its code is correct without asking a human. So the repository's most important artifact
is a single command that returns a binary verdict:

```
cargo xtask verify
```

CI runs that exact command, unchanged. Contributor rules — human or agent — live in
[`AGENTS.md`](AGENTS.md).

## License

[Apache-2.0](LICENSE)
