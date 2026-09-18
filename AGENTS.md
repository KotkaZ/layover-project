# AGENTS.md

Authoritative instructions for any agent working in this repository. This file outranks habit.
If it conflicts with your task prompt, stop and ask.

## What this project is

**Layover** is a local-first Rust framework for running a lights-out agent factory.

It does **not** call LLMs. It is a supervisor: it spawns headless agent CLIs (`claude -p`,
`copilot`, `codex exec`), routes messages between them over MCP, persists their state to disk,
and stops them from running away.

Read `docs/architecture.md` before changing anything structural.

## Project status

**Early implementation.**

What exists and is fully tested:

- `crates/layover-core` — configuration, agents, routes, pipelines, prompt composition, the route
  graph, load-time validation, itinerary accounting (Hops, Fuel, run cap) and rendezvous barriers.
- `crates/layover-http` — the HTTP surface, **generated** from `api/openapi.yaml`. Types, the
  `Api` trait and the axum router.
- `crates/layover-store` — on-disk run history: day-segmented JSON Lines, 90-day retention.
- `crates/layover-dashboard` — the monitoring page and the `Api` implementation behind it.
- `crates/layover-tower` — the supervisor. The only crate that starts a process, and the first
  place in the project that can do something irreversible.
- `crates/layover-mcp` — the MCP surface agents talk to Layover through. Untrusted input arrives
  here; identity comes from the token and never from the request.
- `crates/layover-cli` — the `layover` binary: `validate`, `explain`, `graph`, `prompt`, `serve`,
  `run`, `autostart`.

**`layover run` runs a factory, and agents reach one another.** The supervisor spawns an agent CLI,
serves it an MCP endpoint with a token minted for that run alone, watches it, bounds it and prices
it. An agent that calls `layover_send` queues a real flight and the same invocation runs the next
agent, charging every hop to the one itinerary that began the chain.

What does not exist is anything that starts work on its own: no schedule fires, so a chain has to
be triggered by hand. Barriers at runtime and resuming a booked Layover are also unbuilt.

What each of those will do is settled rather than open: see
[`docs/first-release.md`](docs/first-release.md). What is still genuinely undecided is the short
list in `docs/decisions.md`; do not guess at those.

## The golden rule

```
cargo xtask verify
```

This is the only definition of done. It runs generated-code freshness, documentation checks,
`fmt --check`, `clippy -D warnings`, `test` and the doc build. CI runs this exact command and
nothing else, so a local pass is a CI pass.

That promise depends on `rust-toolchain.toml`, which pins the exact compiler. Clippy gains lints
between releases, so without a pin a contributor on an older toolchain passes locally and fails in
CI — the one failure an unattended agent cannot diagnose. Do not remove the pin. Bumping it is an
ordinary change: raise the version, run `verify`, fix what the newer lints find, commit it all
together.

Never report work complete without running it. Never weaken it to make it pass.

## Documentation is part of the change

**Every change carries its documentation with it. You do not need to be asked.**

Before you call any task finished, work through this list. It is not optional and it is not a
courtesy — an agent that trusts a stale document makes confident wrong changes, and this
repository is meant to be worked on by agents.

| If you changed... | Then update... |
|---|---|
| Anything in `layover.toml`'s shape | `book/src/configuration.md`, both `examples/`, `docs/architecture.md` §6 |
| Cost accounting, Fuel, the Reserve or a rate card | `book/src/cost.md`, `docs/risks.md` risks 4 and 5 |
| Route, join or barrier semantics | `docs/routing.md`, `book/src/configuration.md` |
| Pipelines, triggers or flags | `book/src/pipelines.md`, `examples/workitem-factory/` |
| Prompt composition | `book/src/prompts.md` |
| The HTTP surface | `api/openapi.yaml` (the contract — never edit `generated.rs`), `book/src/http-api.md` |
| A safety rail: Hops, Fuel, run cap, Ground Stop | `docs/architecture.md`, `docs/risks.md`, and the arithmetic in `examples/workitem-factory/README.md` |
| A decision that was not obvious | The decision log in `docs/decisions.md` — *why*, not what |
| Anything listed as open in `docs/decisions.md` | Answer it in the decision log above it, with the reasoning, and delete the question |
| A new known hazard | `docs/risks.md` |
| MCP servers, workspaces or autostart | `book/src/configuration.md`, `book/src/pipelines.md`, `book/src/install.md` |
| Any diagram | Use Mermaid, not ASCII — GitHub and the book both render it |
| Recovery, steering or the handover | `book/src/recovery.md`, `docs/decisions.md` |
| The dashboard, history or cost windows | `book/src/dashboard.md`, `book/src/cost.md` |
| Help requests or learnings | `book/src/learning.md`, `docs/decisions.md` |
| The CLI's commands or flags | `crates/layover-cli/README.md`, `book/src/install.md` |
| How Layover is installed or released | `dist-workspace.toml` then `dist generate`, `book/src/install.md`, both READMEs |

`cargo xtask verify` enforces the parts a machine can check: links that resolve, generated code
that matches its specification, examples that still parse and validate. It cannot tell you whether
a paragraph is still true. That part is yours.

**If a change contradicts something written down, fix the writing in the same commit.** Do not
leave it for later, do not open a follow-up issue, and do not wait to be asked.


## Conventions

- **Conventional Commits.** `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`.
- **Tests are the specification.** Acceptance criteria must be executable. If you cannot write a
  test that fails before your change and passes after, the task is not ready to implement.
- **Keep files under ~500 lines.** Agent file readers truncate large files, and a file that
  cannot be read whole cannot be reasoned about. Split before you exceed it.
- **Crate boundaries are context boundaries.** A crate plus its tests should fit in one agent's
  working context. Split early rather than late.
- **Document why, not what.** The code already says what it does; rationale belongs in `docs/`.

## Terminology

This codebase uses an airport metaphor consistently. These are the canonical names — use them in
type names, API fields and prose alike.

| Term | Meaning |
|---|---|
| **Agent** | A configured agent definition |
| **Flight** | A message in transit between agents |
| **Itinerary** | One causal chain of flights from a single trigger; carries Hops and Fuel |
| **Hops** | Remaining TTL for an itinerary |
| **Fuel** | Remaining token/cost budget for an itinerary |
| **Reserve** | Remaining cost budget for the whole factory, over a rolling window |
| **Tower** | The Layover supervisor process |
| **Run** | One supervised CLI execution |
| **Hangar** | An agent's private state directory |
| **Logbook** | The shared global memory file |
| **Ground Stop** | The kill switch that halts everything |

## Hard rules

1. **Never commit secrets.** API keys reach child CLIs through the environment, never config.
2. **Never weaken a safety rail** — Hops, Fuel and Ground Stop are load-bearing.
3. **Never trust a child agent's claims about its own identity or budget.** Those come from the
   Tower's per-run token. See `docs/architecture.md`.
4. **Never point a factory at this repository's own source.** Explicitly out of scope.
5. **Ask rather than guess** on anything listed as open in `docs/decisions.md`.

## Where things live

| Path | Purpose |
|---|---|
| `api/openapi.yaml` | **The HTTP contract.** Edit this, never `generated.rs`. |
| `dist-workspace.toml` | **Release configuration.** Edit this, never `.github/workflows/release.yml`. |
| `book/` | The published documentation site (mdBook → GitHub Pages) |
| `docs/architecture.md` | System design |
| `docs/decisions.md` | Why the system that exists is shaped as it is, and what is still open |
| `docs/first-release.md` | Decisions made for the first runnable release, not yet built |
| `docs/routing.md` | Route map semantics, joins, failure paths |

| `docs/risks.md` | Known risks and mitigations |
| `examples/workitem-factory/` | The reference v0.1 factory, with its sizing arithmetic |
| `crates/layover-core` | Domain types: config, agents, routes, pipelines, prompts, graph, validation, itinerary, barriers |
| `crates/layover-http` | The generated HTTP surface and the `Api` trait |
| `crates/layover-store` | On-disk run history and retention |
| `crates/layover-dashboard` | The monitoring dashboard: route map, runs, cost |
| `crates/layover-cli` | The `layover` binary |
| `xtask/` | `verify`, `generate-api` and `docs` |
