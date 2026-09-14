# Layover

**A local-first framework for running a lights-out agent factory.**

> **Status: design phase.** No code yet. The design lives in
> [`docs/architecture.md`](docs/architecture.md).

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs, gives them a way
to talk to one another, persists what they learn, and stops them from running away.

You describe a factory in a single `layover.toml` — which agents exist, what each one is for, and
which agents may trigger which others. Layover then runs it unattended.

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
- **Runaway swarms are bounded by construction** — every itinerary burns Hops and Fuel, and the
  Tower, not the agent, holds the counters.

## Scope

v0.1 targets Claude Code, GitHub Copilot CLI and OpenAI Codex CLI, on a single machine.
See [`docs/roadmap.md`](docs/roadmap.md) for what is in and out.

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
