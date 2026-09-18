# Changelog

Notable changes per release. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/).

**Pre-1.0: a minor bump may break things.** Crate versions track releases of what is built; the
*first runnable release* — the milestone where a factory actually runs — has not happened yet.
See [project status](README.md#project-status).

## [Unreleased]

## [0.11.0] — 2026-09-18

`layover run` exists, and it runs things. A flight queued through the API is authorised against the
route map and the safety rails, spawned, watched, priced and written to history.

### Added

- **`layover run`**, with `--dry-run`. Drains the queue once; deliberately not a daemon, because
  nothing routes between agents yet and a command that looped forever would look like a working
  factory that never does anything.
- **Dispatch: deciding whether a flight may fly.** Ground Stop, then route, then rails — in that
  order, because "everything is stopped" should beat a detail about one flight, and "that edge does
  not exist" is permanent where "no Fuel left" is about this chain right now.
- **The rails bite for the first time.** Hops decrement per flight and the count never passes
  through an agent; the run cap counts starts rather than completions, because a run that is never
  seen to finish has still been started; Fuel is debited even when a run fails, because money spent
  is money spent.
- **A flight leaves the queue before it runs**, so a factory that dies mid-run does not repeat the
  work on restart — an agent interrupted after opening a pull request would otherwise open a
  second one.

## [0.10.0] — 2026-09-18

Layover can supervise a run. Not yet a factory — nothing routes a message between agents — but the
mechanisms a supervisor is made of now exist and are tested against real processes.

### Added

- **`layover-tower`: Layover starts a process.** The first code in the project that can do
  something irreversible. A run writes its composed payload to the hangar, builds its invocation,
  spawns the CLI, streams both output streams to one transcript file, and reports how it ended.
- **A run is recorded before it is spawned.** A run alive when the supervisor dies leaves no exit
  code, so the record written first is the only evidence it existed. It carries the process
  identifier *and* the moment it began, because identifiers get reused and recovering into
  something else's process is worse than not recovering.
- **Timeouts, and killing a process tree.** An agent CLI is rarely one process — it starts language
  servers, shells out to git, runs suites — so ending only the child leaves those holding the
  workspace. What that means differs sharply by platform.
- **Stop requests reach a running child.** What makes Ground Stop real rather than advisory.
- **Cost is read from the transcript, and disbelieved when it should be.** A figure an order of
  magnitude below what the reported tokens imply is treated as unreported rather than as a
  measurement, so a runner that under-reports cannot quietly defeat Fuel and the Reserve together.
- **Only declared variables reach a child**, plus the handful the operating system needs for a
  process to exist at all. Clearing the environment outright leaves a child unable to start —
  found the hard way, and now pinned by a test.

### Changed

- The aviation vocabulary stays whole and `entry = true` stays, reversing two earlier decisions.
  Both were simplifications rather than capabilities, and both would have changed config and API
  for factories that already exist.
- The soak gate is 48 hours against a sandbox repository, revised from seven days, and named as a
  compromise: it catches the second-day failures and not the slow ones.

## [0.9.0] — 2026-09-18

The fifty questions blocking the first runnable release were answered, and the run bootstrap they
were blocking is now built.

### Added

- **`payload`: what a run is told, and in what order.** The decision that was blocking the
  supervisor. Identity, instructions, memory, learnings, handover, and the flight body last —
  because whatever arrives last reads as the current instruction, and the body *is* the
  instruction. The order is pinned by a test rather than left to whoever edits next.
- **Memory is injected, not fetched.** The tail of `memory.md` always reaches a run, capped at
  4 KB, saying so when it was cut. Fetch-only would have failed silently: an agent that forgets to
  call for its own notes simply has none, and nothing would report it.
- **`{model}` in runner commands.** Every CLI spells the flag differently, so the spelling stays
  where the invocation does. A bare `"{model}"` argument disappears when no model is set rather
  than becoming an empty string, which several CLIs read as a positional.
- **Validation catches a model that cannot reach its runner** — which immediately found the bug in
  our own shipped examples, where three agents declared a model that would have been ignored.
- **`docs/first-release.md`**: the reasoning behind all fifty answers, including the
  counter-argument wherever the call was close.
- **Branch and tag protection.** `main` requires a passing `cargo xtask verify`; `v*` tags cannot
  be rewritten or deleted.

### Changed

- **`docs/decisions.md` now covers only the system that exists**; decisions made for the first
  runnable release live beside it. Both files are back under the 500-line rule.
- The run-bootstrap questions are gone from the open list, and the source comments in `handover.rs`
  and `prompt.rs` that cited them as open now point at `payload` instead.

## [0.8.0] — 2026-09-17

### Added

- **A logo, status badges and a download table** in the README and on the documentation site,
  with a dark-mode variant and a favicon. [`assets/logo-prep.ps1`](assets/logo-prep.ps1)
  regenerates them from the source artwork.
- **`CONTRIBUTING.md`, `SECURITY.md`, this changelog, a Dependabot configuration and an issue
  template.** The contributor contract already existed in `AGENTS.md`; it was not reachable from
  anywhere a human would look, and there was no private channel for reporting a vulnerability.
- **Redaction on help requests.** `summary` and `detail` are written by an agent explaining why
  something failed, and the commonest reason is a credential — so they are now capped and stripped
  of token-shaped text before they reach disk, the API or the dashboard.
- **A warning when `layover serve` binds off loopback**, since the API has no authentication and
  will queue work for a future supervisor.
- **Trigger a workflow from the dashboard.** A pipeline picker, a prompt, and a switch per
  declared flag. The flight is queued durably and says so — nothing dispatches it until a
  supervisor exists, and a Ground Stop refuses it.
- **Agent reports.** Every run can carry what the agent concluded; the dashboard shows it when a
  run is opened. Capped and trimmed rather than rejected, and a trimmed report says so.
- **One workflow at a time.** A selector scopes the route map, runs, costs and help requests to a
  single pipeline. The Reserve and learnings deliberately do not narrow.
- **The Reserve is now drawn.** It was specified, documented and returned by the API, but never
  shown.
- **A per-workflow activity strip** above each diagram: runs, spend, failures and open help.
- **A CLI reference page** and an `examples/` index.

### Fixed

- **`layover autostart` generated a service that ran `layover run`** — a subcommand that does not
  exist — so it would have failed at every logon while the documentation promised it could not.
  It now generates `serve`, and a test asserts the emitted command parses against the real parser.
- **The Reserve was metered over the window being browsed**, so the default view compared thirty
  days of spend against a twenty-four hour cap.
- **A queued trigger discarded its flags and its pipeline.**
- **`join = "any"` woke its agent once per upstream** rather than exactly once.
- **`[reserve] fuel_usd` accepted negative and NaN values** and silently became unlimited.
- **A claim and its negation counted as the same learning.**
- **A `$0` cost alongside real tokens** was treated as a measurement rather than as silence.
- **`.gitignore` did not cover runtime state outside the repository root**, so running an example
  and staging everything would have committed run history, queued flight bodies and help requests.
- **The install guide claimed both installers verify a checksum.** The shell one skips
  verification when `sha256sum` is absent — which is stock macOS — and the PowerShell one does not
  verify at all. The page now says so and gives a fail-closed alternative.

### Changed

- **`docs/roadmap.md` is gone.** Its open questions moved to `docs/decisions.md`, which also took
  the decision log out of an oversized `docs/architecture.md`.
- **Project status is stated once, in the README.** Three places used to claim different things.
- **The milestone is named rather than numbered.** "v0.1" meant the release where a factory first
  runs, while the crates were at 0.8.0; a goal spelled like a version gets read as one.
- **`rust-version` now matches the pinned toolchain**, because that is the only one tested.
- **CI and Pages pin every action to a commit**, declare least-privilege permissions, and build
  with `--locked`.
- Documentation corrections throughout: the install page claimed nothing was released, offered a
  `cargo install` that 404s, and several figures disagreed with the code.

## [0.7.0] — 2026-09-17

### Added

- **A diagram per workflow**, each showing the trigger, Hops, Fuel, workspace and resume policy
  that bound a chain started there.
- **Cost broken down by workflow**, and a `pipeline` parameter on the graph endpoint.

### Fixed

- **Overlapping edges in the route map.** Layers ignored scope, every edge met a node at its
  centre, and all returns to one agent shared a gutter.

## [0.3.0] — 2026-09-16

### Added

- First tagged release: installers and archives for five targets.

[Unreleased]: https://github.com/KotkaZ/layover-project/compare/v0.11.0...HEAD
[0.11.0]: https://github.com/KotkaZ/layover-project/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/KotkaZ/layover-project/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/KotkaZ/layover-project/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/KotkaZ/layover-project/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/KotkaZ/layover-project/compare/v0.3.0...v0.7.0
[0.3.0]: https://github.com/KotkaZ/layover-project/releases/tag/v0.3.0
