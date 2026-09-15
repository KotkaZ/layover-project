# Layover

**A local-first framework for running a lights-out agent factory.**

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs, gives them a way
to talk to one another, persists what they learn, and stops them from running away.

You describe a factory in a single `layover.toml` — which agents exist, what each one is for,
which agents may trigger which others, and how work gets in. Layover then runs it unattended.

## The idea

Picture an airport grid.

Each **agent** is an airport. A message is a **Flight**. A chain of flights originating from one
trigger is an **Itinerary**, and it carries the two things that keep the network sane: **Hops**
(how many legs remain) and **Fuel** (how much budget remains). The **Tower** is air traffic
control. When everything needs to stop, you call a **Ground Stop**.

## How it works

```text
  you ──POST /flights──▶  Tower  ──spawns──▶  claude -p / copilot / codex exec
                            ▲                          │
                            │                          │ MCP: layover_send(...)
                            └──────────────────────────┘
                                   routes, meters, persists
```

- **Sending a message is what starts an agent.** There is no separate spawn step.
- **Agents talk over MCP.** Claude Code, Copilot CLI and Codex CLI reach Layover natively.
- **Every run is a clean slate.** Nothing carries over implicitly between runs, which makes an
  agent's memory exactly what it chose to write down.
- **The route map is a directed graph.** No edge means the flight is refused.
- **Runaway swarms are bounded by construction** — every itinerary burns Hops and Fuel, and the
  Tower, not the agent, holds the counters.

## Status

Early. What works today is everything that happens *before* the first process is spawned: loading
a factory definition, validating it, and composing prompts. Process supervision, the MCP server
and the HTTP API are specified but not built.

`layover validate` is useful right now, and the [reference factory](./reference-factory.md) is
worth reading even if you never run it.

## Where to start

- [Install](./install.md)
- [Your first factory](./first-factory.md)
- [The reference factory](./reference-factory.md) — the shape v0.1 is sized against
