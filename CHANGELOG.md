# Changelog

Notable changes per release. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/).

**Semantic versioning from 1.0.** A breaking change to the configuration format, the HTTP API, the
MCP tool surface or the on-disk layout requires a major bump. See [project
status](README.md#project-status).

## [Unreleased]

## [1.16.1] — 2026-10-27

A Copilot run that dies at start-up because the model it was given is not available is restarted
like any other run the model service ended, and `layover doctor` names an agent the model service
keeps ending.

**Compatibility.** Additive: every factory that loaded under 1.16 loads unchanged. What an existing
factory will notice: a Copilot run that ends with `Error: Model "…" from --model flag is not
available.` is recorded with `cause: provider_error` and restarted under its agent's `recovery`
policy — `automatic`, the default, up to `max_recovery_attempts` — where before it was the run's own
failure and its chain stopped there. To keep that, set `provider_errors = { read = "none" }` on the
runner or `recovery = "manual"` on the agent. `layover doctor` can now report a warning, and exit
non-zero, for an agent whose last runs were all ended by the model service.

### Fixed

- **A model not available at start-up stopped the chain.** Copilot CLI says so in a line of text,
  not an event, so the run read as its own failure, nothing restarted it, and the work it carried
  sat until a person noticed. The line now counts as the model service's failure — as a whole line
  only, so an agent quoting it never does — and the record keeps the CLI's words, so an outage can
  be told from a mistyped `model`.

### Added

- **`layover doctor` names an agent the model service keeps ending**: one whose last runs — as many
  as one piece of work may take, and at least two — were all `provider_error`, with what the CLI
  said the last time.

## [1.16.0] — 2026-10-21

The route map is laid out in reading order, so a workflow with shortcuts and loops reads along its
main path; and the documentation is reorganised around what a person looks for: a first factory
that runs on one CLI, pages for updating and troubleshooting, the design inside the book, and a site
that describes the release you can install.

**Compatibility.** Nothing a factory does changes. Its route map — on the dashboard and from
`layover graph --svg` — may look different: columns follow the order its routes are declared in
rather than hop distance, so listing routes roughly in the order work flows keeps the map readable.
`layover validate` and `layover explain` say exactly what they said. The first factory,
`examples/planner.toml`, now runs every agent on Copilot CLI instead of Claude Code, Copilot CLI
and Codex; a factory copied from it before is unaffected. Links to sections of `configuration.html`
about runners, routes and updating, and of `dashboard.html` about chains, runs and cost, now land at
the top of the page that held them: those sections have pages of their own. For code that uses
`layover-core` as a library, `diagram::Edge` gains a `via` field.

### Changed

- **The route map is laid out in reading order, not hop distance.** Columns come from a depth-first
  walk from the way in, visiting routes in the order `layover.toml` declares them; a route that
  closes a loop is drawn back. A production workflow whose analyst may send straight to agents
  several phases on had five agents in one column; it now reads analyst, consults, reviewer,
  developer, tester, operator, messenger. An edge that skips columns crosses each one clear of its
  boxes, a route back runs just below the columns it spans rather than under the whole map, and
  each agent's tooltip gives its hop depth as `layover validate` measures it. A sweep that spawns
  its reviewer now draws the reviewer's consults after it rather than beside the sweep's entry.
- **The documentation recommends a CLI home of the factory's own** — `COPILOT_HOME`,
  `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, forwarded with `env_from` — so a factory's sessions stay out of
  the user's history and its agents never load the user's personal plugins and MCP servers. The
  install guide says how a variable reaches a Tower `autostart` starts; risk 30 records the hazard.
- **The first factory needs one CLI.** `examples/planner.toml` asked for three CLIs and an
  `ANTHROPIC_API_KEY`, and its Codex reviewer could not reach Layover, so the loop broke at its
  third agent. Every agent now uses the `copilot` preset.
- **The site's root is the latest release's book**, rebuilt when a release is published; `main`'s
  is at `/dev/`, with a banner on every page saying it may describe what no release does yet.
- **New pages:** Updating Layover, Troubleshooting (by the message you see), Glossary, Runners,
  Routes and joins, and the dashboard's Chains and Runs views, each split from a page that had grown
  past 500 lines. The design documents — architecture, routing, the decision log and the risks —
  are pages of the book, written from `docs/` when it is built.
- The decision log is split by topic into `docs/decisions/`, and the risks of releasing and updating
  into `docs/risks-releases.md`. Cutting a release moved from the install guide to `RELEASING.md`.
- The README is shorter, and the book's Status is the one list of what is and is not built. The
  CHANGELOG's older sections are cut to bullets, their Compatibility paragraphs unchanged.
- `cargo xtask verify` checks `#anchors` as well as links, absolute links to the repository and the
  site, that every `docs/` file is a page of the book, and that every CLI command and flag,
  `layover.toml` key and HTTP endpoint is in its reference page. `cargo xtask book` writes the
  design pages.
## [1.15.0] — 2026-10-19

A Tower that is asked to stop now lets what it is running finish, and Layover can update itself: it
hears of a new release, gets it ready, verified and checked against the factory, and — asked —
installs it between runs and restarts on it, rolling back one that cannot prove itself.

**Compatibility.** Additive for `layover.toml` and the HTTP API: every factory that loaded under
1.14 loads unchanged, and the one new table, `[update]`, is optional. What an existing factory will
notice:
- **The first Ctrl+C on `layover serve` drains** instead of exiting at once: nothing new starts, what
  is running finishes, and then it exits. A second Ctrl+C cuts what is running off and exits.
- **The Tower makes one request a day to github.com**, for the newest release's manifest, and says
  when a newer release exists. It downloads and changes nothing. `[update] mode = "off"` stops it
  for a factory and `LAYOVER_UPDATE=off` for a machine.
- **`layover doctor` begins with a line** saying which Layover runs and what this machine last
  learned about releases.
- **One Tower serves a factory.** A second `serve` on a factory another is serving refuses to start,
  where it used to race the first for its queue.
- **The service files `layover autostart` writes for systemd and launchd set
  `LAYOVER_SUPERVISOR`**, so a Tower installing a release exits 75 for the service to restart it.
  Regenerate an older one; a systemd service is recognised without it.
- **`validate` warns when `[update] max_drain` is shorter than a run's `timeout`**, which the
  defaults never are.
- **The HTTP API**: `GET /health` gains `stopping`, and `/update` is new, with `apply`, `skip` and
  `rollback` beneath it.

### Added

- `layover stop` tells the Tower serving a factory to drain and exit; `--now` cuts off its runs.
- Cutting off — a second Ctrl+C or `--now` — kills each run's process tree, for the next Tower to
  record as `interrupted`. Every way of stopping exits 0.
- A draining Tower starts nothing new and holds schedule ticks, except work completing a rendezvous
  that holds an arrival. The version pill reads `· stopping`.
- `[update]`: `mode` (`notify` by default, or `off`), `channel` (`stable` or `prerelease`) and
  `check_every` (`"24h"`). What is found is kept per machine.
- A newer release shows on the version pill (`v1.14.0 → 1.15.0`), with its notes, and in `doctor`.
- `layover update check` looks now; `layover update skip <version>` stops offering one, and
  `--clear` undoes it.
- `layover about` (`--json`) prints a binary's version, platform, state layout, HTTP API and the
  oldest release it can be updated from.
- `[update] mode = "stage"` or `layover update stage` downloads the newest release (up to 64 MB),
  checks its SHA-256 and `gh attestation verify`, and validates the factory with it.
- Nothing downloaded runs unverified; without `gh`, `--trust-checksum-only` trusts the checksum.
- `layover update apply`, or **Update now**, drains for up to `[update] max_drain` (`"4h"`), then
  cuts off what still runs for recovery to restart on the new version.
- A running `recovery = "manual"` agent postpones an install. The old binary is kept for
  `[update] keep_previous` (`"7d"`), and exit codes say how it went.
- On Windows the new Tower is started and the old binary restored if it does not answer; under
  systemd or launchd the Tower exits 75. The new one logs to `.layover/serve.log`.
- With no Tower serving, `apply` replaces the binary at once; a Homebrew or npm install is left to
  its package manager.
- `[update] mode = "auto"` installs a patch or minor release (`allow`) once `min_age` old
  (`{ patch = "24h", minor = "48h" }`), verified (`require_provenance`) and validating cleanly.
- It installs in the `window` (`"Sat 02:00-06:00"`, local) or after `idle_for` (`"10m"`) idle — at
  the next window if this one closes first. Major and state-layout releases wait for a person.
- The dashboard's bell rings for a new release, an update and a rollback.
- `layover update rollback` returns to the replaced version; `layover update status` says where
  things stand; `layover update log` prints `.layover/updates.jsonl`.
- `GET /update` (`staged`, `gate`, `request`, `last_update`, `previous`); `POST /update/apply`,
  `DELETE /update/apply`, `POST /update/skip` and `POST /update/rollback`.
- `LAYOVER_UPDATE`, `LAYOVER_UPDATE_SOURCE` (a mirror laid out as GitHub's releases) and
  `LAYOVER_GITHUB_TOKEN`.

### Tried

- Released first as 1.15.0-rc.1 and rc.2 on GitHub only. A throwaway factory installed, rolled back
  and auto-installed them; what they turned up is fixed here.

## [1.14.0] — 2026-10-14

A run the model service ended is restarted like an interrupted one, and a chain that ends on a
failed run no longer reads as finished.

**Compatibility.** Additive for `layover.toml`: every factory that loaded under 1.13 loads unchanged,
and the one new key, a runner's `provider_errors`, is optional. What an existing factory will notice:
- **A Copilot run the model service ended is restarted**, after a minute, where its agent's
  `recovery` is `automatic` — the default — up to `max_recovery_attempts` (default `2`). It is on by
  default because `recovery` already says whether an agent's work is safe to repeat. To keep the old
  behaviour, set `max_recovery_attempts = 0`, `recovery = "manual"` on an agent, or
  `provider_errors = { read = "none" }` on a runner. No other failed run is restarted.
- **Chains that ended on a failed, timed-out or interrupted run show as `failed`**, in Chains'
  *needs attention*, the tab's count, the page title and notifications — including ones that ended
  that way before this release, within the window shown.
- **`layover run` waits** for a restart that is backing off rather than stopping with it queued.
- **The HTTP API**: `ItineraryState` gains `failed` and `ItineraryList` a `failed` count; a run gains
  `cause` and `request_id`; a queued flight gains `attempt` and `not_before`. A client that treats
  the chain state as a closed set should expect the new value.
- **Run records** in `runs-*.jsonl` gain `cause` and `request_id`, absent from older records.
- **Going back to 1.13 or earlier**: a restart this release queued, or one alive when the Tower
  stopped, is not understood by them and would be dropped. Let it start, or cancel it, first.

### Added

- A Copilot run that failed because its last model call did — a `result` not `success`, or a
  `session.error` of type `query` — records `cause: provider_error` and `request_id`.
- It is restarted under the agent's `recovery` and `max_recovery_attempts` after one, five, then
  fifteen minutes, told the earlier run's tools may have acted. **Upcoming** shows when.
- `provider_errors` on a runner: `read = "copilot"`, `read = "none"`, or `failed` and `carried_on`
  events for another CLI. `validate` refuses one that cannot decide.
- The `failed` chain state: its last run failed, timed out or was interrupted, with nothing after.

## [1.13.0] — 2026-10-13

A dashboard for a factory that has been running for weeks: lists that open on what needs a person
and page through the rest, addresses for every view, pages that keep themselves current, a chain
drawn against the clock, spend day by day, and a tab title that says when something is waiting.

**Compatibility.** Additive. `GET /costs` gains `by_day` and `day_zone`; nothing is removed or
renamed. Links to `#chain=<id>` keep working.

### Added

- Every view has an address with its filters (`#runs?status=failed&workflow=eagle-eye`); changing
  tab is a browser history step.
- **Chains** opens on what needs attention, can be searched and can hide a schedule's finished
  chains; it pages by fifty.
- **Runs** pages by a hundred and shows each run's chain, model and effort; **Learnings** filter by
  standing, agent and text.
- **Trigger…** beside each workflow on the route map; the window names the agent the prompt goes to.
- The route map, **Chains** and **Runs** refresh every 15 s and say when they last read the Tower.
- With three or more workflows, idle ones start folded and the busiest first; a choice is kept.
- A chain's page draws its runs on one time axis, with non-default flags as chips.
- **Cost** shows spend day by day, the costliest chains and the Reserve's pace; `GET /costs` gains
  `by_day` and `day_zone`.
- The tab title counts what needs you (`(3) Layover`); a bell turns on browser notifications.
- **Sessions** opens the newest running session; a terminal links to its chain and report.

### Changed

- The Ground Stop stays on screen at any width; below 1180 px the tabs get a row of their own.
- Diagrams are drawn at their natural size, and a box title too long for it is fitted, not clipped.
- Times read `3 h ago` everywhere, exact on hover; a running run says `running · started 4 min ago`.
- **Continue…** moved to a chain's own page, and is not offered for a workflow that only resumes.
- The route map's queue line is amber only when queued work has nothing to start it.

## [1.12.0] — 2026-10-12

`layover.toml` says more with less: flags and a follow-up's routes are declared once, lengths of
time read as durations, a workflow sizes its own chains, and the numbers learnings live by are the
factory's — with lapsed learnings deleted rather than kept for ever.

**Compatibility.** Additive for the file format: every factory that loaded under 1.11 loads and
runs, and new keys are optional. What an existing factory will notice:
- **Lapsed learnings are deleted** after `keep_lapsed_days`, fourteen by default, counted from when
  each lapsed — or, for one that lapsed before this release, from when it was last proposed. The
  first `layover serve` deletes any already past that. Set `[learnings] keep_lapsed_days` higher
  first to keep more.
- **An agent's `fuel_usd` is applied** to chains starting at it, as documented; see Fixed.
- **`layover serve` binds `[layover] http_addr`** when `--addr` is not given. A factory that set it
  to something other than the default, believing it ignored, now listens there.
- **`validate --strict` fails on `[layover] state_dir`**, which is warned about now.
- **The HTTP API**: `Pipeline.entry` may be `null`, for a resuming pipeline that names none — only
  for factories that use that; `Learning` gains `lapsed_at` and `deleted_after`.

### Added

- `layover serve` binds `[layover] http_addr`, which was ignored; `--addr` still wins, and the
  default is `127.0.0.1:7878`.
- Durations such as `timeout = "2h"` in `[defaults]` and on a route, `[reserve] window = "24h"` and
  `[learnings] keep_lapsed = "14d"`; the `_sec`, `_hours` and `_days` keys take them too.
- Naming both forms of a key is an error, and a duration that is not whole hours or days, where a
  key counts in them, is refused.
- `[learnings]`: `provisional_runs` (`20`), `confirm_after` (`3`), `max_chars` (`400`) and
  `max_per_run` (`25`); `validate` refuses `0`.
- Lapsed learnings are deleted after `[learnings] keep_lapsed_days` (`14`; `0` at once), checked
  after each run and when `serve` starts. A rediscovery after that starts a new learning.
- `GET /learnings` and the dashboard give a lapsed learning's `lapsed_at` and `deleted_after`.
- A top-level `[flags]` table declares flags for every workflow; `validate` warns about a workflow
  repeating one.
- `routes_from = "development"` on a pipeline gives it that pipeline's scoped routes, transitively.
- `max_hops`, `fuel_usd`, `max_runs` and `timeout` on `[pipelines.<name>]` replace `[defaults]` for
  its chains; `validate` refuses zero. The Reserve stays factory-wide.
- A pipeline that `resumes` may leave out `entry`, and then cannot be triggered by hand;
  `GET /pipelines` gives `entry: null`.

### Changed

- `validate` warns about `[layover] state_dir`, never honoured; `validate --strict` fails on it.

### Fixed

- An agent's `fuel_usd` now applies to chains starting at it; they all ran on `[defaults] fuel_usd`.

## [1.11.0] — 2026-10-12

Any workflow can be switched off — its schedule included — while the rest of the factory runs.

**Compatibility.** Additive. Factories load, validate and run unchanged, and nothing is switched off
until somebody switches it off. `GET /pipelines` gains `switched_off` and `switched_off_since`,
`GET /flights` gains `held`, and `GET /upcoming` gains `switched_off`. An older release ignores
`.layover/journal/switched-off.jsonl`, so under it every workflow runs as though switched on.

### Added

- A switch beside each workflow on the route map, or `POST /pipelines/{name}/switch-off` (`DELETE`
  to undo), switches it off. Chains under way finish.
- Off, its ticks pass unfired, a trigger gets `409`, a resuming workflow collects nothing, and
  queued triggers are `held`.
- Back on, held triggers start and the schedule fires at its next tick, not making up missed ones.
- The switch survives restarts in `.layover/journal/switched-off.jsonl`; `layover doctor` names
  workflows that are off.

## [1.10.0] — 2026-10-08

A runner says only what its agents are denied: for Copilot CLI and Claude Code, Layover supplies the
rest. A workflow can run a shared agent at its own model, effort or context, and whoever triggers a
run can name it and choose those for that run alone.

**Compatibility.** Additive. Existing factories — written-out `command` runners included — load,
validate and run unchanged, and `validate` warns about nothing new in them. What is different:
- A written-out `command` containing the literal text `{name}` or `{args}` now has it filled in —
  the chain's name, the agent's `args` — or left out when there is none.
- `POST /flights` takes optional `name` and `agents`; `GET /pipelines` gains each workflow's
  `agents`, itineraries and pending flights a `name`, and runs a `chain_name`. Run records, queued
  flights, layovers and help requests keep the chain's name and choices where it has them; older
  releases ignore them, and older records simply lack them.

### Added

- Runner presets `cli = "copilot"` and `cli = "claude"` supply the program, model, effort, context,
  JSON output, MCP wiring, and Copilot's `--no-ask-user` and `--name`.
- The Copilot preset passes `--allow-all-tools --allow-all-paths --allow-all-urls`, so a runner's
  `args` say what it denies. `command` still works.
- An agent's own `args` are added to its runner's command, at `{args}` or at the end.
- `[pipelines.<name>.agents.<agent>]` sets the model, effort, context and extra `args` a workflow
  runs an agent at, over the agent's values and `[defaults]`.
- A trigger can name its run and choose a model, effort or context per agent for it (`name` and
  `agents` on `POST /flights`). A choice the runner cannot carry is refused.
- The name and choices follow everything the chain causes. The name shows on **Chains**,
  **Sessions**, **Upcoming** and **Runs** (`chain_name`) and is Copilot's `--name`.
- Maps, `layover explain`, `prompt --pipeline` and run records show what each agent ran at;
  `GET /pipelines` lists each workflow's `agents`.
- `layover validate` refuses a runner with neither or both of `cli` and `command`, `args` beside a
  `command`, and an override for an undeclared agent.
- It warns about an option a preset already supplies, `args` fixing a placeholder's value or after a
  final `-`, and an override no chain reaches or the runner cannot carry.
- `examples/multi-workflow/`: four workflows sharing eight agents and one Copilot runner.

### Changed

- The examples use presets, which give Copilot agents `--allow-all-paths`, `--allow-all-urls` and
  `--no-ask-user`; Claude runners gain the `--verbose` they lacked.

## [1.9.0] — 2026-10-07

An agent declares its own reasoning effort and context tier beside its model, so one runner serves
every agent with the same permissions; and every run records which effort and context it ran at.

**Compatibility.** Additive. Existing factories — including ones whose runners fix an effort or a
context in their command — load, validate and run unchanged; `validate` warns about nothing new
unless a runner both fixes a value and has the placeholder for it. Two things are different in an
existing factory:
- A runner whose `{model}` an agent leaves unset no longer hands the CLI a flag without its value,
  or the literal `{model}`; the argument is left out, with its option. Such runs failed at spawn.
- Run records gain `effort` and `context`, and `GET /runs` gains `reasoning_effort` and `context`;
  older releases and records simply lack them.

### Added

- An agent's `effort` and `context` reach its runner through `{effort}` and `{context}`, as
  `{model}` does; `[defaults] effort` and `[defaults] context` fill in.
- `layover validate` warns when an effort or context cannot reach its runner, a runner fixes one
  beside its placeholder, or a value is empty or unused.
- Run records carry `effort` and `context`, `GET /runs` `reasoning_effort` and `context`, and a
  session's header shows them.
- `layover explain` says what each agent runs on; `layover prompt` prints it on stderr.

### Fixed

- An argument with an unset `{model}` is left out with its option, instead of passing `--model` bare
  or the literal `{model}`.

### Changed

- For library users: `Runner::invocation` and `invocation_with_mcp` take a `Selection`; `Runner`
  moved to `config/runner.rs`.

A rate card is still keyed by model alone, so it is a lower bound for long-context runs billed at a
higher rate. Copilot runs are unaffected.

## [1.8.0] — 2026-10-07

What a factory will do next is on the dashboard: every scheduled tick by the Tower's own clock,
every layover with when it will actually be picked up, the queue, and the ticks that were skipped.

**Compatibility.** Additions only. The configuration format and the MCP tools are unchanged; the
HTTP API gains an endpoint and the on-disk layout a file, which an older release ignores. For
anyone using the crates as a library, `layover_dashboard::Dashboard::timetable` and the
`Timetable` trait are new.

### Added

- An **Upcoming** tab: queued work (with **Cancel**), scheduled ticks for the next 6 hours to 7
  days, layovers and when they will be picked up, and ticks skipped in the last 7 days.
- A tick held by a Ground Stop, or by its workflow's still-running previous wave, is marked.
- The route map shows `next 14:00 · in 23 min` beside each schedule; its strip counts skipped ticks.
- `GET /upcoming?hours=`; a `--watch-only` server answers `clock: null` and no ticks.
- Skipped ticks are kept 90 days in `.layover/journal/skips-<day>.jsonl`.

## [1.7.1] — 2026-10-05

The documentation describes Layover as it is, and the code nothing used is gone. Layovers never
backed off or expired, although the book said they did; when to stop waiting is up to the waiting
agent's prompt.

**Compatibility.** The configuration format, the HTTP API, the MCP tools and the on-disk layout
are unchanged; a layover is still written with `checks` and `max_checks`, so a factory can go back
to 1.7.0. Two things are different in an existing factory:
- A resumed run's brief says when the work was set down, and no longer "this is check 1".
- `layover doctor` no longer has an "expired layovers" finding. It could never appear.

### Removed

- Library code nothing used, outside the semantic-versioning promise: `layover_core::slots`,
  `Factory::run_flight`, unreported-cost counters, and layover back-off and expiry.

### Fixed

- Documentation from before 1.0 — `SECURITY.md`, the README, `AGENTS.md`, the book,
  `docs/architecture.md` — describes 1.7.0 and what is not built yet.
- `SECURITY.md` no longer advises relying on `access`, and `layover run --help` no longer says there
  is no MCP server.
- `[layover] state_dir` is documented as not honoured; `planner.toml` and the reference factory drop
  it and `http_addr`.
- The book no longer says layovers back off and expire. The reference factory's follower checks
  hourly, then daily, and stops after a week.

## [1.7.0] — 2026-10-02

Spend nobody reported no longer reads as zero, and Layover says why a runner's costs are missing and
what to add to its command.

**Compatibility.** The configuration format, the HTTP API, the MCP tools and the on-disk layout
are unchanged. Three things behave differently in an existing factory:
- `layover validate` has a new warning, so a factory checked with `--strict` whose Copilot or
  Claude Code runner lacks its JSON output now fails the check, as it should: its spend has been
  unknowable.
- A factory with a `[rates]` table now gets estimates for runs that print tokens and no dollars,
  shown as `rate_card` and never debited from Fuel or the Reserve.
- A run's recorded `model` is the one its command line selects, falling back to the declared one,
  so **By model** gains runner-fixed models.

### Added

- `layover validate` warns about a runner that cannot report cost: Copilot without
  `--output-format json`, Claude Code without JSON output, Codex without `--json` under a rate card.
- `layover doctor` says, runner by runner, why runs reported no cost and what to change.

### Fixed

- `[rates.<model>]` now prices a run that prints tokens and no dollars, as `rate_card`: shown and
  totalled, never debited from Fuel or the Reserve.
- A total built partly from unreported runs carries a `+`, and one built wholly from them reads
  **not reported**, not `$0.00`.
- `examples/planner.toml` runs Copilot with `--output-format json`.

## [1.6.0] — 2026-10-02

See one run of a workflow on its own, even while the same workflow runs several times at once.

**Compatibility.** The configuration format is unchanged. The HTTP API only gains:
`GET /itineraries/{itinerary_id}`, `sent_by` on a run, and `running` and `queued` on a chain. Two
things behave differently in an existing factory. `GET /itineraries` now lists a chain from the
moment its first flight is queued — `working`, with `runs: 0` — so a client that assumed every
chain has a run will meet one that has none. And a workflow's route map is coloured by that
workflow's runs: an agent it shares with another workflow no longer shows as running because the
other workflow is running it. On disk, run records and live-run records gain an optional `sent_by`;
everything an earlier release wrote still reads, and its runs simply light no routes. For library
users, `RunRecord`, `layover_store::live::Live`, `diagram::Live`, `diagram::Node` and
`diagram::Edge` gain fields, and `diagram::Activity` gains `Done` and `Queued`.

### Added

- A chain's own page draws its workflow's map for that chain alone — `×2` on an agent that ran
  twice, its routes in green — at `#chain=itn_…`.
- Below the map, each run with its sender, outcome, cost, **Watch**/**Transcript** and **Report**,
  then queued work. Triggering a workflow opens its chain.
- Under a workflow's map, a button per running chain says where it is (`46TNEG at coder`); an agent
  with several live runs shows `×2`.
- **Chains** says `working · at coder` or `working · queued`.
- Run records gain `sent_by`: the sending agents, `[]` for outside work, `null` where not recorded.
- `GET /itineraries/{itinerary_id}`: a chain, its runs, its queued flights and its map.

### Fixed

- A chain triggered while every slot was taken is on **Chains** as soon as it is queued.
- A workflow's map no longer shows an agent running because another workflow is running it.

## [1.5.1] — 2026-10-01

Dependency and CI maintenance. Nothing a factory sees changes.

**Compatibility.** The configuration format, the HTTP API, the MCP tools and the on-disk layout are
unchanged, and identifiers keep their shape. For library users, `ConfigError::Parse` carries a
`toml` 1.x `toml::de::Error` where it carried a 0.9 one, so code that names that type needs
`toml` 1.

### Changed

- `ulid` 3.0, `toml` 1.1, `rand` 0.10 and `thiserror` 2.0.21; identifiers and `layover.toml` syntax
  are unchanged.
- Workflow actions updated; Dependabot leaves the `dist`-generated `.github/workflows/release.yml`.

## [1.5.0] — 2026-10-01

Watch agents work from the dashboard, live, the way their CLI would show it; answer an agent's help
request and have the work continue with its chain's flags; and see which chains are waiting for you.

**Compatibility.** The configuration format is unchanged. The HTTP API only gains: `POST
/help/reply`, the `awaiting_human` chain state, `flags`, `reply`, `waiting_for`, `continues` and
`continued_by` where they apply, and `GET /runs/{run_id}/stream`, which answered `501` and now
streams. A client that treats `ItineraryState` as a closed set will see a value it does not know.
On disk, help requests, run records and queued flights gain optional fields, so anything an
earlier release wrote still reads. Two things behave differently in an existing factory: running
runs now appear in **Runs**, **Chains** and on the route map, where they never did; and a run lost
under 1.3.0 is settled with when it was last seen alive. For library users, `Api` gains
`reply_help`, `stream_run` takes a `StreamRunQuery`, and `Ledger`/`Live` moved to
`layover_store::live` (still exported by `layover_tower`).

### Added

- **Reply** on a help request runs the asking agent again in a new chain with a fresh budget and the
  same workflow, flags and routes, beginning `In reply to your help request …`.
- `POST /help/reply` does it with the trigger's token: `409` for a request already dealt with, `404`
  for an unknown one.
- The `awaiting_human` state ("waiting for you") marks a chain stopped on an open fatal help
  request, with **Reply…** and an amber count on **Chains**.
- `GET /itineraries` filters on it; chains gain `flags`, `waiting_for`, `continues`, `continued_by`.
- **Continue…** on a chain or report opens the trigger window with that chain's workflow and flags.
- Help requests and run records keep the chain's flags; `GET /help` and `GET /runs` show them.
- A **Sessions** tab shows running runs, and those that ended in the last day, as read-only live
  terminals: prompt, reasoning, tool calls and replies.
- **Tile running sessions**, **Show thinking**, **Follow**, and **Watch**/**Transcript** on runs.
  Credentials are masked and output cut to its first lines.
- `GET /runs/{run_id}/stream`: server-sent `entry`, `partial` and `end` events; `?after=` resumes.

### Fixed

- **Resolved** no longer implies the work will continue, and points to **Reply**.
- A run 1.3.0 left behind ends when its transcript was last written; any run found gone after a
  restart ends when last seen alive.
- A help request filed while another is resolved is no longer lost. Empty "0" badges are hidden.

### Changed

- For library users: `HelpRequest`, `RunRecord` and `Queued` gain optional fields.

## [1.4.0] — 2026-09-30

The Tower runs agents in parallel, prices Copilot runs from their AI credits, enforces the Reserve,
and settles the runs a restart left behind.

**Compatibility.** The configuration format, the HTTP API and the on-disk layout only gain:
`[copilot] usd_per_credit` and `[agents.<name>] max_concurrent` in `layover.toml`;
`copilot_credits`, `credit_runs`, `alive_runs` and `max_concurrent_runs` in the API; `queued_at` on
run records, and the work and owning Tower on a live run's record. Everything that loaded before
still loads. Three things behave differently in an existing factory: **runs overlap**, up to
`max_concurrent_runs` at once — `1` restores one at a time; **Copilot runs are priced**, so
`fuel_usd` now cuts Copilot chains; and **the Reserve is enforced**, including its default of $100
in any 24 hours for a factory that sets none. History written with `copilot_credits` is not counted
by an earlier release. For library users, `Wiring`'s learnings hooks and `from_transcript`'s
signature changed.

### Changed

- The Tower runs up to `max_concurrent_runs` agents at once, factory-wide, starting the oldest
  queued flight that can start; `layover run` too.
- Agents sharing a `work_dir` can now write it at once, and runs admitted together can overshoot
  Fuel or the Reserve by up to `max_concurrent_runs` runs.
- History files a run under the day it finished.
- Copilot runs are priced from their last `session.usage_checkpoint` at `[copilot] usd_per_credit`,
  as `copilot_credits`, and debit Fuel and the Reserve.
- The Reserve is enforced: past `[reserve] fuel_usd` a run is refused and recorded `halted`.
  `fuel_usd = 0` means no ceiling; a raised cap needs a restart.

### Added

- `[copilot] usd_per_credit` (default `0.01`); `validate` refuses zero, negative and non-finite.
- `copilot_credits` in `CostSource`, and `credit_runs` on every `CostSummary`.
- `[agents.<name>] max_concurrent` caps one agent's live runs. `validate` refuses `0`, and warns
  that `max_concurrent_runs = 0` is read as 1.
- `GET /flights` gains `dispatched_by`, `alive_runs` and `max_concurrent_runs`; the dashboard shows
  `2 of 4 run(s) alive · 3 flight(s) queued`.
- `layover doctor` warns about Reserve refusals, an exhausted Reserve, and work that waited with a
  slot free. Run records carry `queued_at`.
- For library users: `from_transcript` takes the credit rate; `Wiring` has one `update_learnings`.

### Fixed

- A run alive when the Tower went away is stopped by the next Tower, recorded `interrupted`, and
  restarted under its agent's `recovery` and `max_recovery_attempts`.
- Runs writing at once no longer lose each other's `memory.md` notes, learnings, logbook entries,
  help requests, reports or queue.
- A rendezvous is not given up while its upstream is still queued.
- On Linux, a run that times out or meets a Ground Stop is actually killed; procps 4.0.4's `kill`
  misread the old call.

## [1.3.0] — 2026-09-30

The route map shows each agent's reasoning effort beside its model.

**Compatibility.** Nothing in the configuration format, the HTTP API, the MCP tools or the
on-disk layout changes. Only the drawing does: an agent's first line under its name now reads
`model · effort …` when its command line sets an effort, and a box may have a third small line.

### Added

- The route map shows an agent's effort beside its model (`claude-opus-5.5 · effort xhigh`); the
  tooltip and pinned panel say "effort".

## [1.2.0] — 2026-09-29

The route map says which model each agent runs on, draws a busy workflow without the tangle, and
lets you trace one agent's routes at a time.

**Compatibility.** Nothing in the configuration format, the MCP tools or the on-disk layout
changes. `GET /agents` gains `reasoning_effort` and `context`, and its `model` is now the model the
agent's command line selects rather than only the one it declares — the same type, no longer
`null` for an agent whose runner fixes its model. The drawn map changes shape: each route is a
`<g class="route">` naming its ends, a pair of plain routes is one line, and a scoped route's
tooltip leads with its ends before `only in …`. Anything that scraped the old SVG will need to
look again; `layover graph` without `--svg` prints the same Mermaid as before.

### Added

- The route map shows each agent's model and context tier, read from its command line; `GET /agents`
  reports `model`, `reasoning_effort` and `context`.
- Opposite plain routes are one double-headed line, and same-column routes short connectors.
- Hovering an agent lights its routes; clicking pins a panel with its model, runner, access, peers
  and runs. Every route has a tooltip.

### Fixed

- A return path no longer runs behind or through the boxes it is routed around.

## [1.1.0] — 2026-09-29

Workflows that share agents can now keep their routes apart, and nine defects found running a real
factory on Windows are fixed — three of them stopped a factory doing what its configuration said.

**Compatibility.** Everything new is optional: `pipelines` on a route and on the HTTP `Route`,
`blocker` on `layover_help`, and `layover graph --pipeline`. Queued work and layovers written by
1.1 still read in 1.0.0. Two validation errors are new, each for a factory that could not run as
written: an agent MCP server named `layover`, which would collide with Layover's own; and a
spawned agent that tests a flag its spawning pipeline does not declare, which now inherits that
pipeline's flags instead of borrowing another pipeline's default. One warning is no longer
suppressed: `workspace = "per-itinerary"` isolates nothing yet, so a schedule that can overlap its
own writers is warned about, and `validate --strict` fails on it.

### Added

- `pipelines = "name"` (or a list) on `[[routes]]` scopes a route to those pipelines' chains, for
  `layover_send` and `layover_peers`. Unscoped routes are global.
- A spawned chain inherits its pipeline, a resumed layover is held to its booking chain's reach, and
  joins apply only in their scope.
- `validate` checks reach, hops, joins and flags per pipeline, refuses unknown or empty `pipelines`
  and conflicting overlaps, and warns about unusable routes.
- Each workflow's map, `GET /graph?pipeline=` and `layover graph --pipeline` show only its routes;
  `GET /agents` and `layover explain` give scopes.

### Fixed

- Flags chosen at trigger time were ignored. A chain's flags now reach every run it causes, across
  restarts, spawns and layovers.
- A relative `--config`, the default included, broke every run. An agent's relative `work_dir`
  resolves against the factory.
- A failed run records its exit code and likeliest error line, redacted, capped at 300 characters.
- `[agents.<name>.mcp.<server>]` servers reach the run, credentials staying in the environment;
  `validate` refuses one named `layover`.
- Codex MCP wiring and `env_from` for HTTP servers cannot work yet; both are open questions.
- `layover_help` takes a `blocker` category. Help requests and reports go to the journal, read by
  the dashboard, `doctor` and `GET /reports/{run}`.
- Every run is told who sent its flight, in a `== WHO SENT THIS ==` section.
- A resumed layover gets the message that woke its booking run and that run's last `layover_report`,
  each cut to 2,000 characters.
- `access = "read-only"` and per-itinerary workspaces are documented as declared, not enforced;
  `layover explain` says so.

## [1.0.0] — 2026-09-23

Layover runs a factory unattended for two days without being touched: the bar this project set for
itself, now cleared rather than asserted.

### The evidence

- A 48-hour soak on 0.23.3, 2026-09-21 08:47 to 2026-09-23 08:47 UTC, one process, untouched: 1,501
  of 1,501 runs succeeded.
- 1,453 heartbeat runs every 2 minutes and 24 two-agent chains on the real Copilot CLI; 12.6 MB
  working set after 48 hours.

### Fixed

- Hangars are pruned at 90 days, except an agent's `memory.md` and directories Layover did not mint.

### What 1.0 means

- A breaking change needs a major bump; Hops, Fuel, the run cap, the Reserve and Ground Stop are a
  stable contract.
- Copilot CLI reports no cost, so only `max_runs` and `timeout_sec` bind a Copilot factory.

## [0.23.3] — 2026-09-21

`prompt_file` never reached a run. This is the most serious bug found so far.

### Fixed

- An agent defined with `prompt_file`, as in every shipped example, ran with no instructions. Its
  prompt and flags now reach the run, and an unresolvable prompt refuses it.
- The `` You are `name`. `` placeholder is no longer sent twice.

## [0.23.2] — 2026-09-21

### Fixed

- On Windows, agents can run `git`, `npm` and the like through PowerShell: the child environment
  carries `PATHEXT`.
- `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` and `SystemDrive` reach the child, so a CLI finds
  credentials it already has.

## [0.23.1] — 2026-09-21

### Fixed

- The Homebrew formula passes `brew style`, and the release is on crates.io, as 0.23.0 was not.

## [0.23.0] — 2026-09-21

Installable the way people actually install things.

### Added

- Homebrew: `brew install KotkaZ/tap/layover` on macOS and Linux, from a
  [tap](https://docs.brew.sh/Taps), latest version only.
- crates.io: `cargo install layover-cli` — not `layover`, an unrelated crate with a binary of that
  name — and the library crates.

### Fixed

- The book no longer says Layover is not on crates.io.

## [0.22.0] — 2026-09-20

The reference factory ran end to end on a real agent CLI for the first time, and found three things
every test had passed over.

### Fixed

- `env_from` on an agent and in `[defaults]`, combined, names the variables that give an agent CLI
  its credentials; a run missing one is refused.
- Copilot's MCP wiring uses `--additional-mcp-config` with the new `prefix = "@"`; the flag the
  examples named did not exist.
- `docs/architecture.md` no longer shows `-p {prompt}`, which passed the prompt's path as its text.
- `layover doctor` reports a layover waiting where no pipeline has `resumes = true` as a fault.

### Documented

- Copilot CLI reports no cost, only `premiumRequests`, so Fuel cannot bind a Copilot factory;
  `layover doctor` says so.

## [0.21.0] — 2026-09-19

A soak you cannot check is not proof. This adds the command that checks it.

### Added

- `layover doctor` reports stalled chains, costs built from silence, schedules that never fired, an
  engaged Ground Stop and open help requests. Faults and warnings exit non-zero.

### Fixed

- `layover-cli`'s README and the CLI reference describe the commands that exist.

## [0.20.0] — 2026-09-19

The last of the engineering before 1.0. What remains is proof, not code.

### Security

- The API and page require a token by default, printed in the address at startup and kept in a
  `SameSite=Strict`, `HttpOnly` cookie; `Authorization: Bearer` and `?token=` work too.
- `--no-auth` turns it off, and warns beside a non-loopback bind.

### Added

- Signed build provenance for every artifact, and an attested CycloneDX SBOM with each release.

### Fixed

- `--no-auth` no longer refuses every request.

## [0.19.0] — 2026-09-19

Hardening the two channels that outlive a run: learnings and prompt files.

### Security

- A learning that tries to override instructions, names Layover's tools, carries a URL or looks like
  a credential is refused. Learnings are quoted, on one line, in the prompt.
- The prompt sandbox compares resolved paths, so a symlink in the prompt directory cannot escape it.

### Fixed

- The credential check no longer fires on paths, commit SHAs and ordinary text.

## [0.18.0] — 2026-09-19

The factory remembers. All ten tools are connected, and a run is finally given what earlier runs of
it knew.

### Fixed

- An agent's `memory.md` and learnings reach its runs; they never did. A provisional learning ages
  with every run, whatever its outcome.

### Added

- `layover_learn` answers with what became of a proposal; `layover_logbook_append` stamps each entry
  with who wrote it and when.

## [0.17.0] — 2026-09-19

Layover has layovers: `layover_wait`, a declared tool that answered "not connected yet" for six
releases, now works.

### Added

- `layover_wait` sets work down for a wait such as `30m`, `2h` or `3d`, and the run ends; nothing
  stays alive in between.
- A `resumes = true` pipeline, and no other, collects due layovers, each as a new chain with a fresh
  budget and the context of what was set down.

### Fixed

- Booking a layover no longer races the Tower resuming one.

## [0.16.2] — 2026-09-19

**Identical in content to 0.16.0.** 0.16.1 hand-edited the generated release workflow to fix a build
that had not failed, and its own build failed; the workflow is back to its generated form.

## [0.16.1] — 2026-09-19

Tagged; the build failed. No release exists.

## [0.16.0] — 2026-09-19

The factory can ask you things, and now you can answer. The state directory is versioned, so two
releases cannot silently disagree about it.

### Added

- Help requests can be resolved from the API or a button, by run, agent or blocker; an agent that
  hits the same wall raises it again.
- `PATCH /learnings/{id}` with `confirmed` keeps a learning, and `rejected` drops it from every
  future run. Learnings never wait for approval.
- `.layover/version.json` records the state directory's layout: a newer one is refused, an older one
  migrated once.

### Fixed

- A route-map test that depended on the date.

## [0.15.0] — 2026-09-18

You can now see what the factory is doing and stop it from the page you are watching it on.

### Added

- The Ground Stop works over HTTP, with a dashboard button; engaging it twice succeeds.
- `GET /itineraries` and a **Chains** tab (`working`, `finished`, `stalled`, `halted`);
  `DELETE /flights/{id}` cancels work not yet started.

### Fixed

- Runs carry the pipeline that opened their chain; they recorded `pipeline: null`.

## [0.14.0] — 2026-09-18

The factory runs itself. `layover serve` fires scheduled pipelines, runs what is queued, and serves
agents the endpoint they call back into — so a chain starts, travels and finishes with nobody
watching.

### Added

- `layover serve` is the Tower — schedules, queue and MCP endpoint; `--watch-only` only watches.
- `trigger = { every = "1h" }` or `{ cron = "0 8,18 * * *" }` fires by the clock, once after a sleep
  and never at startup.
- A tick is skipped and reported while its last wave runs, unless `overlap = "allow"`.
- A joined agent wakes once, when its last declared upstream arrives; a rendezvous nothing can
  complete is given up and named.

### Fixed

- `mode = "spawn"` opens a fresh itinerary with its own Hops, Fuel and run cap; it was ignored. The
  queue no longer loses concurrent updates.
- `layover validate` no longer warns that a scheduled pipeline can overlap itself, which the default
  now prevents.

## [0.13.0] — 2026-09-18

Agents reach one another. A run is served an MCP endpoint it can call back into, and a chain is no
longer one hop long.

### Added

- `layover run` serves an MCP endpoint and a per-run token; `layover_send` queues a flight the same
  drain runs, on the chain's one itinerary.
- The route map is enforced on the live run, a token dies with its run, and an unknown token gets
  HTTP 401. `{mcp}` places the MCP flag in a command.

### Fixed

- A drain whose every flight is refused no longer spins.

## [0.12.0] — 2026-09-18

The MCP surface agents talk to Layover through: the protocol, the tool registry, and a check that
stops a prompt naming a tool that does not exist.

### Added

- JSON-RPC 2.0 `initialize`, `tools/list` and `tools/call` with ten tools; identity comes from the
  token, and a refused call is an `isError` result.
- `layover validate` refuses a prompt that names a tool Layover does not offer.

## [0.11.0] — 2026-09-18

`layover run` exists, and it runs things. A flight queued through the API is authorised against the
route map and the safety rails, spawned, watched, priced and written to history.

### Added

- `layover run`, with `--dry-run`, drains the queue once, checking Ground Stop, route and rails in
  that order; a flight leaves the queue before it runs.
- Hops, the run cap and Fuel are enforced for the first time; Fuel is debited even for a failed run.

## [0.10.0] — 2026-09-18

Layover can supervise a run. Not yet a factory — nothing routes a message between agents — but the
mechanisms a supervisor is made of now exist and are tested against real processes.

### Added

- `layover-tower` spawns a CLI, records the run first, writes its transcript, kills its process tree
  on timeout or stop, and reads its cost.
- A cost far below the reported tokens counts as unreported. Only declared variables, and what the
  OS needs, reach a child.

### Changed

- The aviation vocabulary and `entry = true` stay. The soak gate is 48 hours, not seven days.

## [0.9.0] — 2026-09-18

The fifty questions blocking the first runnable release were answered, and the run bootstrap they
were blocking is now built.

### Added

- A run's payload in a fixed order — identity, instructions, memory, learnings, handover, flight
  body — with the last 4 KB of `memory.md`.
- `{model}` in runner commands; `validate` catches a model that cannot reach its runner.
  `docs/first-release.md` records the answers.

### Changed

- `docs/decisions.md` covers only the system that exists.

## [0.8.0] — 2026-09-17

### Added

- A logo, badges and a download table; [`assets/logo-prep.ps1`](assets/logo-prep.ps1) regenerates
  them. `CONTRIBUTING.md`, `SECURITY.md` and this changelog.
- Triggering a workflow from the dashboard, agent reports, a workflow selector, the Reserve drawn,
  and per-workflow activity strips.
- Help request text is capped and redacted, and `layover serve` warns when it binds off loopback.

### Fixed

- `layover autostart` runs `serve`. The Reserve is metered over its own window; `[reserve] fuel_usd`
  refuses negative and NaN.
- A queued trigger keeps its flags and pipeline, `join = "any"` wakes its agent once, and a `$0`
  cost beside real tokens is unreported.
- A claim and its negation are separate learnings, `.gitignore` covers runtime state, and the
  install guide says which installers verify a checksum.

### Changed

- `docs/roadmap.md` is gone; its open questions are in `docs/decisions.md`.

## [0.7.0] — 2026-09-17

### Added

- A diagram per workflow, cost by workflow, and a `pipeline` parameter on the graph endpoint.

### Fixed

- Overlapping edges in the route map.

## [0.3.0] — 2026-09-16

### Added

- First tagged release: installers and archives for five targets.

[Unreleased]: https://github.com/KotkaZ/layover-project/compare/v1.16.1...HEAD
[1.16.1]: https://github.com/KotkaZ/layover-project/compare/v1.16.0...v1.16.1
[1.16.0]: https://github.com/KotkaZ/layover-project/compare/v1.15.0...v1.16.0
[1.15.0]: https://github.com/KotkaZ/layover-project/compare/v1.14.0...v1.15.0
[1.14.0]: https://github.com/KotkaZ/layover-project/compare/v1.13.0...v1.14.0
[1.13.0]: https://github.com/KotkaZ/layover-project/compare/v1.12.0...v1.13.0
[1.12.0]: https://github.com/KotkaZ/layover-project/compare/v1.11.0...v1.12.0
[1.11.0]: https://github.com/KotkaZ/layover-project/compare/v1.10.0...v1.11.0
[1.10.0]: https://github.com/KotkaZ/layover-project/compare/v1.9.0...v1.10.0
[1.9.0]: https://github.com/KotkaZ/layover-project/compare/v1.8.0...v1.9.0
[1.8.0]: https://github.com/KotkaZ/layover-project/compare/v1.7.1...v1.8.0
[1.7.1]: https://github.com/KotkaZ/layover-project/compare/v1.7.0...v1.7.1
[1.7.0]: https://github.com/KotkaZ/layover-project/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/KotkaZ/layover-project/compare/v1.5.1...v1.6.0
[1.5.1]: https://github.com/KotkaZ/layover-project/compare/v1.5.0...v1.5.1
[1.5.0]: https://github.com/KotkaZ/layover-project/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/KotkaZ/layover-project/compare/v1.3.0...v1.4.0
[1.3.0]: https://github.com/KotkaZ/layover-project/compare/v1.2.0...v1.3.0
[1.2.0]: https://github.com/KotkaZ/layover-project/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/KotkaZ/layover-project/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/KotkaZ/layover-project/compare/v0.23.3...v1.0.0
[0.23.3]: https://github.com/KotkaZ/layover-project/compare/v0.23.2...v0.23.3
[0.23.2]: https://github.com/KotkaZ/layover-project/compare/v0.23.1...v0.23.2
[0.23.1]: https://github.com/KotkaZ/layover-project/compare/v0.23.0...v0.23.1
[0.23.0]: https://github.com/KotkaZ/layover-project/compare/v0.22.0...v0.23.0
[0.22.0]: https://github.com/KotkaZ/layover-project/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/KotkaZ/layover-project/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/KotkaZ/layover-project/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/KotkaZ/layover-project/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/KotkaZ/layover-project/compare/v0.17.0...v0.18.0
[0.17.0]: https://github.com/KotkaZ/layover-project/compare/v0.16.2...v0.17.0
[0.16.2]: https://github.com/KotkaZ/layover-project/compare/v0.16.1...v0.16.2
[0.16.1]: https://github.com/KotkaZ/layover-project/compare/v0.16.0...v0.16.1
[0.16.0]: https://github.com/KotkaZ/layover-project/compare/v0.15.0...v0.16.0
[0.15.0]: https://github.com/KotkaZ/layover-project/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/KotkaZ/layover-project/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/KotkaZ/layover-project/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/KotkaZ/layover-project/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/KotkaZ/layover-project/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/KotkaZ/layover-project/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/KotkaZ/layover-project/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/KotkaZ/layover-project/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/KotkaZ/layover-project/compare/v0.3.0...v0.7.0
[0.3.0]: https://github.com/KotkaZ/layover-project/releases/tag/v0.3.0
