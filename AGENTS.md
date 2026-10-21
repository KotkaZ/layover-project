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

**Released, and running unattended**; semantic versioning applies from 1.0. The current version is
in the workspace `Cargo.toml`, what each release added in `CHANGELOG.md`, and what Layover does
today — and does not do yet — in the book's [Status](book/src/index.md#status), which is the one
place that says so. Two gaps matter whenever you change something: there is no isolation between
agents — `access` and `workspace` are declared and every agent still shares `work_dir` — and Codex
cannot yet be wired to the MCP endpoint.

What each crate is for is under [Where things live](#where-things-live). Why the system is shaped
as it is lives in the decision log — `docs/decisions.md` and the topic files it lists in
`docs/decisions/` — and what is still genuinely undecided is the short list at the end of
`docs/decisions.md`; do not guess at those. [`docs/first-release.md`](docs/first-release.md)
records the decisions made before 1.0, with their reasoning.

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
| Anything in `layover.toml`'s shape | `book/src/configuration.md` (every key is in its tables; what a runner, route or `[update]` key does is in `runners.md`, `routes.md` or `updating.md`), every example in `examples/` that uses it, `docs/architecture.md` §6 |
| Cost accounting, Fuel, the Reserve or a rate card | `book/src/cost.md`, `docs/risks.md` risks 4 and 5 |
| Route, join or barrier semantics | `docs/routing.md`, `book/src/routes.md` |
| Pipelines, triggers or flags | `book/src/pipelines.md`, `examples/workitem-factory/` |
| Prompt composition | `book/src/prompts.md` |
| The HTTP surface | `api/openapi.yaml` (the contract — never edit `generated.rs`), `book/src/http-api.md` |
| A safety rail: Hops, Fuel, run cap, Ground Stop | `docs/architecture.md`, `docs/risks.md`, and the arithmetic in `examples/workitem-factory/README.md` |
| A decision that was not obvious | The topic file it belongs to in `docs/decisions/`, listed in `docs/decisions.md` — *why*, not what |
| Anything listed as open in `docs/decisions.md` | Answer it in the decision log, with the reasoning, and delete the question |
| A new known hazard | `docs/risks.md`, or `docs/risks-releases.md` for releases and updating |
| MCP servers, workspaces or autostart | `book/src/configuration.md`, `book/src/pipelines.md`, `book/src/install.md` |
| What a run's environment carries: `env_from`, the scrubbed variables, a CLI's home | `book/src/runners.md` (*A CLI home of the factory's own*), `book/src/install.md`, `docs/risks.md` risk 30 |
| Any diagram | Use Mermaid, not ASCII — GitHub and the book both render it |
| Recovery, steering or the handover | `book/src/recovery.md`, `docs/decisions/recovery.md` |
| The dashboard, history or cost windows | `book/src/dashboard.md` and its two pages, `dashboard-chains.md` and `dashboard-runs.md`; `book/src/cost.md` |
| Help requests or learnings | `book/src/learning.md`, `docs/decisions/learning.md` |
| The CLI's commands or flags | `book/src/cli.md`, `crates/layover-cli/README.md`, `book/src/install.md` |
| How Layover is installed or released | `dist-workspace.toml` then `dist init --yes`, `RELEASING.md`, `book/src/install.md`, both READMEs |
| How Layover updates itself: `[update]`, `layover update`, the gate | `book/src/updating.md`, the `[update]` table in `book/src/configuration.md`, `docs/decisions/updates.md`, `docs/risks-releases.md` |
| A message a person sees when something fails | `book/src/troubleshooting.md`, if it is one people will search for |
| A term in the vocabulary below | `book/src/glossary.md`, and the [Terminology](#terminology) table |
| A file in `docs/` added, renamed or removed | `book/src/SUMMARY.md`, which lists it as `./design/…` — the book's design pages are written from `docs/` |

`cargo xtask verify` enforces the parts a machine can check: links and `#anchors` that resolve,
every `docs/` file listed in the book, every command and flag in `cli.md`, every `layover.toml` key
in the configuration reference and every endpoint in `http-api.md`, generated code that matches its
specification, examples that still parse and validate. It cannot tell you whether
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
type names, API fields and prose alike. The book's [glossary](book/src/glossary.md) defines them,
and the rest of the vocabulary, for people using Layover.

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

## Adding an HTTP endpoint

`api/openapi.yaml` is the contract; `cargo xtask generate-api` turns it into the server. Three
things then need updating by hand, and the compiler only finds them one at a time:

1. The `Api` implementation in `crates/layover-dashboard/src/api.rs`.
2. The `Stub` in `crates/layover-http/tests/contract.rs`, and its `OPERATIONS.len()` assertion.
3. **The doctest in `crates/layover-http/src/lib.rs`** — it implements `Api` by hand, and a warm
   doctest cache hides the breakage locally until CI finds it. Check it before pushing.

## Cutting a release

[`RELEASING.md`](RELEASING.md) is the whole process; follow it. Two things in it have each cost a
failed release: the version lives in **four places**, which `verify` finds one at a time; and
**`.github/workflows/release.yml` is generated** — never hand-edit it; change `dist-workspace.toml`
and run `dist init --yes`.

## Testing against a real agent CLI

Shell stand-ins prove the supervisor; they do not prove the wiring. Three defects survived a full
test suite because every test used `cmd /c echo`: no agent CLI could authenticate, the Copilot MCP
flag in every example did not exist, and Copilot reports no cost at all.

Test against the real binary, in a throwaway repository under the temp directory — **never against
this repository's own source**. `layover doctor` reports what the run left behind.

## Where things live

| Path | Purpose |
|---|---|
| `api/openapi.yaml` | **The HTTP contract.** Edit this, never `generated.rs`. |
| `dist-workspace.toml` | **Release configuration.** Edit this, never `.github/workflows/release.yml`. |
| `RELEASING.md` | **How a release is cut**, for maintainers |
| `book/` | The published documentation site (mdBook → GitHub Pages): the latest release at the root, `main` at `/dev/` |
| `book/src/design/` | **Generated** from `docs/` by `cargo xtask book`; never committed. Edit `docs/`. |
| `docs/architecture.md` | System design |
| `docs/decisions.md` | The decision log's index, and what is still open |
| `docs/decisions/` | Why the system that exists is shaped as it is, one file per topic |
| `docs/first-release.md` | Decisions made before 1.0, with their reasoning |
| `docs/routing.md` | Route map semantics, joins, failure paths |
| `docs/risks.md`, `docs/risks-releases.md` | Known risks and mitigations; the second for releases and updating |
| `examples/workitem-factory/` | The reference factory, with its sizing arithmetic |
| `crates/layover-core` | Domain types: config, agents, routes, pipelines, prompts, graph, validation, itinerary, barriers |
| `crates/layover-http` | The generated HTTP surface and the `Api` trait |
| `crates/layover-store` | On-disk run history, the journal, Hangars and retention |
| `crates/layover-dashboard` | The monitoring dashboard: route map, chains, sessions, runs, cost |
| `crates/layover-tower` | The supervisor: dispatch, rails, recovery — the only crate that starts a process, and the first place that can do something irreversible |
| `crates/layover-mcp` | The MCP endpoint agents call back into. Untrusted input arrives here; identity comes from the token, never the request |
| `crates/layover-update` | Release discovery and staging: manifest, versions, download, provenance, gate. Makes the only network requests Layover itself makes |
| `crates/layover-cli` | The `layover` binary: `validate`, `explain`, `graph`, `prompt`, `serve`, `stop`, `run`, `doctor`, `autostart`, `update`, `about` |
| `xtask/` | `verify`, `generate-api`, `docs`, and `book`, which writes the design pages |
