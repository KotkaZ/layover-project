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

**Design phase. No Rust code exists yet.**

Do not scaffold crates, add dependencies, or write implementation code unless the task explicitly
asks for it.

## The golden rule

```
cargo xtask verify
```

This is the only definition of done. It runs `fmt --check`, `clippy -D warnings`, `test` and the
doc build. CI runs this exact command and nothing else, so a local pass is a CI pass.

Never report work complete without running it. Never weaken it to make it pass.

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
5. **Ask rather than guess** on anything listed as open in `docs/roadmap.md`.

## Where things live

| Path | Purpose |
|---|---|
| `docs/architecture.md` | System design and the decision log |
| `docs/routing.md` | Route map semantics, joins, failure paths |
| `docs/roadmap.md` | v0.1 scope and open questions |
| `docs/risks.md` | Known risks and mitigations |
| `crates/` | Rust workspace — not yet created |
| `xtask/` | The `verify` command — not yet created |
