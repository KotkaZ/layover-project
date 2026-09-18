<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="./assets/logo-dark.png">
  <img src="./assets/logo.png" alt="Layover" width="420">
</picture>

**A local-first framework for running a lights-out agent factory.**

[![CI](https://github.com/KotkaZ/layover-project/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/KotkaZ/layover-project/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/KotkaZ/layover-project?label=download&color=1f6feb)](https://github.com/KotkaZ/layover-project/releases/latest)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](https://github.com/KotkaZ/layover-project/blob/main/LICENSE)

</div>

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs, gives them a way
to talk to one another, persists what they learn, and stops them from running away.

You describe a factory in a single `layover.toml` — which agents exist, what each one is for,
which agents may trigger which others, and how work gets in. Layover then runs it unattended.

## The idea

Picture an airport grid.

Each **agent** is an airport. A message is a **Flight**. A chain of flights originating from one
trigger is an **Itinerary**, and it carries the two things that keep the network sane: **Hops**
(how many legs remain) and **Fuel** (how much budget remains). The **Tower** is air traffic
control. One supervised CLI execution is a **Run**; each agent keeps its own notes in its
**Hangar** and shares what everyone should know in the **Logbook**. Work an agent sets down to
pick up later is a **Layover**. When everything needs to stop, you call a **Ground Stop**.

## How it works

```mermaid
flowchart LR
    you([you]) -->|POST /flights| tower[Tower]
    tower -->|spawns| cli["claude -p<br/>copilot<br/>codex exec"]
    cli -->|"MCP: layover_send(...)"| tower
    tower -.->|routes · meters · persists| store[("hangars<br/>logbook")]

    classDef t fill:#eaf2fb,stroke:#3f6fa3,color:#12263a
    class tower t
```

- **Sending a message is what starts an agent.** There is no separate spawn step.
- **Agents talk over MCP.** Claude Code, Copilot CLI and Codex CLI reach Layover natively.
- **Every run is a clean slate.** Nothing carries over implicitly between runs, which makes an
  agent's memory exactly what it chose to write down.
- **The route map is a directed graph.** No edge means the flight is refused.
- **Runaway swarms are bounded by construction** — every itinerary burns Hops and Fuel, and the
  Tower, not the agent, holds the counters.

## Status

Early. A factory loads, validates and composes its prompts, and `layover serve` puts a dashboard
on it — route map, run history, cost and the Reserve, help requests, learnings and each agent's
report.

**`layover serve` runs the factory.** It fires scheduled pipelines, drains the queue, spawns agent
CLIs, watches them, times them out if they wedge, reads what they cost and writes each run to
history — and serves the MCP endpoint they call back into.

**Agents reach one another.** Each run gets a token minted for it alone. An agent that calls
`layover_send` queues a real flight; the same drain picks it up and runs the next agent. Every hop
is charged to the one itinerary that began the chain, so Hops, Fuel and the run cap bound the whole
conversation rather than each message in it. The route map is enforced against the live child.

**Work waits at a rendezvous.** A joined agent's flights are parked, and it wakes once, with every
verdict it was waiting for. A barrier nothing can complete is given up and named rather than left
to hang.

**What is not proven is two days unattended**, which is the bar this project set for itself. All of
the above is tested and has been watched working; none of it has been left alone.

The [README](https://github.com/KotkaZ/layover-project#what-works-today) carries the built and
not-built list, kept in one place so the two cannot disagree.

## Where to start

- [Install](./install.md) — a single binary, no toolchain needed
- [Your first factory](./first-factory.md) — three agents, and the commands that work today
- [The dashboard](./dashboard.md) — `layover serve`, the most useful thing here right now
- [The reference factory](./reference-factory.md) — the shape the first runnable release is sized against

## Download

The [latest release](https://github.com/KotkaZ/layover-project/releases/latest) carries builds for
Linux x86-64 and ARM64, macOS Intel and Apple silicon, and Windows x86-64, with a `sha256.sum`
covering every artifact. The [install page](./install.md) has the one-line installers.
