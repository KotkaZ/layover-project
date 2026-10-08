# Changelog

Notable changes per release. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/).

**Semantic versioning from 1.0.** A breaking change to the configuration format, the HTTP API, the
MCP tool surface or the on-disk layout requires a major bump. See [project
status](README.md#project-status).

## [Unreleased]

## [1.10.0] — 2026-10-08

A runner says only what its agents are denied: for Copilot CLI and Claude Code, Layover supplies
the rest. A workflow can run a shared agent at its own model, effort or context, and whoever
triggers a run can name it and choose those for that run alone.

**Compatibility.** Additive. Existing factories — written-out `command` runners included — load,
validate and run unchanged, and `validate` warns about nothing new in them. What is different:
- A written-out `command` containing the literal text `{name}` or `{args}` now has it filled in —
  the chain's name, the agent's `args` — or left out when there is none.
- `POST /flights` takes optional `name` and `agents`; `GET /pipelines` gains each workflow's
  `agents`, itineraries and pending flights a `name`, and runs a `chain_name`. Run records, queued
  flights, layovers and help requests keep the chain's name and choices where it has them; older
  releases ignore them, and older records simply lack them.

### Added

- **Runner presets.** `cli = "copilot"` or `cli = "claude"` has Layover supply everything an
  unattended run of that CLI needs — the program, the agent's model, effort and context, Copilot's
  `--no-ask-user` and `--name`, the JSON output a cost is read from, and the MCP wiring — and, for
  Copilot, `--allow-all-tools --allow-all-paths --allow-all-urls`, since nobody is there to answer a
  permission prompt. A runner's `args` then say only what it takes away. `command` still works, for
  Codex, a script, or a runner that wants none of a preset's defaults.
- **An agent's own `args`**, added to its runner's command for that agent alone, so an agent that
  is denied one more thing than the rest needs no runner of its own. They go where a command has
  `{args}`, or at the end.
- **A workflow can run an agent differently.** `[pipelines.<name>.agents.<agent>]` sets the model,
  effort or context that workflow's chains run the agent at, and `args` it adds, so an agent shared
  by two workflows needs neither a second agent nor a second runner. It wins over the agent's own
  values and `[defaults]`; a choice made when triggering wins over it. Each workflow's map, `explain` and
  `prompt --pipeline` show the agent as that workflow runs it, and run records name what it ran on.
- **A run can be named when it is triggered**, and agents run differently for that run only. The
  dashboard's trigger window takes a name — "Login page: retry banner" — and, per agent the workflow
  runs, a model, effort or context to use instead; `POST /flights` takes them as `name` and
  `agents`. Both stay with everything the chain causes: hand-offs, spawned chains, follow-ups, and
  the work a reply to its help requests continues. The name is shown on Chains, the chain's page,
  Sessions, Upcoming and Runs, kept in run records as `chain_name`, and passed to Copilot as the
  session's `--name`. A choice its runner cannot carry, for an agent the workflow never runs, or
  that is not a plain value is refused with the reason. `GET /pipelines` lists each workflow's
  agents with what they run at in it, and a chain's own map shows what its agents ran at in it.
- **`layover validate`** refuses a runner with neither `cli` nor `command`, both, or `args` beside a
  `command`; and warns when a runner or agent repeats an option its preset already supplies, when
  an agent's `args` fix a value its runner carries a placeholder for, and when `args` would follow a
  command's final `-`; refuses a workflow override for an undeclared agent and warns about one no
  chain of the workflow can reach or whose value the runner cannot carry. An agent that leaves a
  preset's effort or context unset is not warned about: the CLI's own default is what it asked for.
- **`examples/multi-workflow/`**: four workflows sharing eight agents and one Copilot runner, shaped
  after a team's factory — a manually triggered development workflow, a follow-up that resumes its
  published pull requests, a review sweep that spawns one review per pull request and runs the shared
  reviewer at a lower effort with one more deny rule, and a daily upstream watch.

### Changed

- **The examples use presets.** `news-digest`, `pr-review`, `build-and-review` and
  `workitem-factory` run Copilot and Claude Code through `cli`, so their Copilot agents now also get
  `--allow-all-paths --allow-all-urls --no-ask-user`. `planner.toml` keeps written-out `command`
  runners for Copilot and Codex, to show that shape. The examples' Claude runners previously lacked
  the `--verbose` Claude Code requires beside `stream-json` in print mode.

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

- **An agent declares its own `effort` and `context`**, beside its `model`, and a runner carries
  them with `{effort}` and `{context}` placeholders exactly as `{model}` carries a model —
  `"--reasoning-effort={effort}"`, `"--context={context}"`, or Codex's
  `"-c", "model_reasoning_effort={effort}"`. A runner now describes a CLI and a permission set, and
  is shared by every agent with those permissions whatever its model, effort and context: tuning one
  agent no longer means a new runner and a copied deny list. `[defaults] effort` and
  `[defaults] context` fill in for agents that set none; an agent's own value wins. Values are
  passed through as written — Layover keeps no catalog of the levels a model accepts.
- **`layover validate` warns** when an agent's effort or context has no placeholder to reach its
  runner, when a runner with a placeholder serves an agent that sets no value and has no default,
  when a runner fixes a value beside its own placeholder (the CLI would get two), when a
  `[defaults]` value reaches no runner, and when a value is empty.
- **Every run records the model, effort and context it ran with**, in `runs-*.jsonl` as `effort`
  and `context`, and in `GET /runs` as `reasoning_effort` and `context`; a session's header shows
  them. History says which effort a run used without its transcript. Older records lack the fields.
- **`layover explain` says what each agent runs on**, and `layover prompt` prints it on stderr
  beside the prompt, both read from the command line with the agent's own values filled in.

### Fixed

- **An unset `{model}` no longer hands the CLI a flag without its value.** `"--model", "{model}"`
  with no model left `--model` in front of the next flag, which Copilot CLI refuses outright, and a
  joined `--model={model}` reached the CLI as that literal text. An argument carrying an unset
  placeholder is now left out whole, together with the option it is the value of. The reference
  factory's Copilot and Codex agents, which declare no model, were affected.

### Changed

- For anyone using the crates as a library: `Runner::invocation` and `invocation_with_mcp` take a
  `Selection` instead of a model, `spawn::Plan` carries a `selection`, and `cost::of_run` returns a
  `ModelChoice`. `Runner` and `McpWiring` moved to `config/runner.rs` and are still reached as
  `layover_core::config::Runner`.

A rate card is still keyed by model alone. A provider that bills a long-context tier at a higher
rate above a token threshold is not modelled, so a card for such a model is a lower bound on its
long-context runs; Copilot runs are priced from the credits they report and are unaffected.

## [1.8.0] — 2026-10-07

What a factory will do next is on the dashboard: every scheduled tick by the Tower's own clock,
every layover with when it will actually be picked up, the queue, and the ticks that were skipped.

**Compatibility.** Additions only. The configuration format and the MCP tools are unchanged; the
HTTP API gains an endpoint and the on-disk layout a file, which an older release ignores. For
anyone using the crates as a library, `layover_dashboard::Dashboard::timetable` and the
`Timetable` trait are new.

### Added

- **The dashboard shows what is going to start, and when.** A new **Upcoming** tab lists, in the
  order it will happen: work queued for a slot, with **Cancel**; every tick of every scheduled
  workflow in the next 6 hours to 7 days, marking a tick held by a Ground Stop and the next tick of
  a workflow whose previous run is still going; every layover waiting, with when it is due and when
  a resuming workflow will actually pick it up; and the ticks skipped in the last seven days. The
  route map gains **next 14:00 · in 23 min** beside each scheduled workflow's trigger, and its strip
  a count of skipped ticks. Until now none of this was visible: a trigger said `every 1h` and not
  when, layovers were not shown at all, and a skipped tick was a line on the Tower's console.
- **`GET /upcoming?hours=`**, which the tab reads. The times come from the clock of the Tower in the
  same process, because an `every` schedule counts from when that Tower started; a
  `--watch-only` server answers with `clock: null` and no ticks rather than guessing.
- **Skipped ticks are written down**, to `.layover/journal/skips-<day>.jsonl`, pruned on the same
  90-day horizon as the rest of the journal.

## [1.7.1] — 2026-10-05

The documentation describes Layover as it is, and the code nothing used is gone. Layovers never
backed off or expired, although the book said they did: it now says what waiting does, and when to
stop is up to the waiting agent's prompt.

**Compatibility.** The configuration format, the HTTP API, the MCP tools and the on-disk layout
are unchanged; a layover is still written with `checks` and `max_checks`, so a factory can go back
to 1.7.0. Two things are different in an existing factory:
- A resumed run's brief says when the work was set down, and no longer "this is check 1".
- `layover doctor` no longer has an "expired layovers" finding. It could never appear.

Anyone using the crates as a library loses the items listed under **Removed**. That API is outside
the semantic-versioning promise.

### Removed

- **Library code nothing used**, for anyone depending on the crates. None of it changes what a
  factory does: the configuration, the HTTP API, the MCP tools and the on-disk layout are untouched,
  which is what the semantic-versioning promise covers.
  - `layover_core::slots` (`Slots`, `Admission`). It predated the dispatcher, which since 1.4.0 has
    counted the runs it holds alive instead, and nothing ever used it.
  - `layover_tower::Factory::run_flight`, the one-flight-at-a-time path from before 1.4.0. Nothing
    but its own tests took it; they now run their flight through `drain`, as `serve` does.
    `Dispatched` gains `Clone`.
  - The itinerary's count of runs that reported no cost (`Itinerary::note_unreported_cost`,
    `has_cost_reporting_gap`, `unreported_runs`, `metered_share`). Nothing read it; that a run
    measured nothing is carried by its record's cost source, which is what the dashboard, the
    costs API and `layover doctor` read.
  - `HelpRequest::is_same_ask_as`, never wired to anything; `Ledger::for_itinerary` and
    `Summary::is_fully_measured` (use `confidence().is_measured()`); `diagram::Live::is_idle`;
    `Factory::env_for`.
  - Layover back-off and expiry: `Layover::set_down_again`, `cancel`, `minutes_until_due`, the
    `Expired` and `Cancelled` standings, and `book`'s `max_checks` argument. Nothing outside their
    tests ever called them, so no layover was ever expired or cancelled; `layover doctor` loses the
    "expired layovers" finding that could therefore never appear. A layover's `checks` and
    `max_checks` are still written, as 0, because releases up to 1.7.0 need them to read it back.

### Fixed

- **Documentation that described Layover as it was before 1.0.** `SECURITY.md` said the project
  was pre-1.0 and that nothing spawned a process; the README said both "Status: 1.0" and "Pre-1.0:
  expect breaking changes on a minor bump"; `AGENTS.md`, the book's front page and the reference
  factory's walkthrough said the Tower, its MCP endpoint or the 48-hour soak did not exist yet; the
  install page pinned `v0.8.0` and said releases carried no build provenance; and
  `docs/architecture.md` described a disk layout, Flight envelope, tool list and crate tree from
  the original design. All now describe 1.7.0, and say plainly what is still not built: worktree
  isolation for `access` and `workspace`, resident agents, and Codex MCP wiring.
- **`layover run --help` said there was no MCP server**, so agents could not hand work on. There
  is, and work they hand on runs in the same drain.
- **`[layover] state_dir` was documented as where Hangars live.** It is parsed and not honoured —
  everything is kept in `.layover/` beside `layover.toml` — and now says so. `planner.toml` and the
  reference factory no longer set it or `http_addr`, neither of which does anything.
- **`SECURITY.md` advised bounding agents with `access`**, which is declared and not enforced. It
  now says not to rely on it.
- **Layovers were documented to back off and expire, and never did.** The book said each fruitless
  check pushed the next one further out, from fifteen minutes to six hours, and gave up after
  twelve. In every release a resumed run that finds nothing books a *new* layover, for whatever
  wait it asks for, and nothing stops it asking again. The book and `docs/decisions.md` now say
  that, and that when to stop waiting belongs in the waiting agent's prompt; the reference
  factory's follower now checks hourly, then daily, and stops after a week. A resumed run's brief
  no longer says "this is check 1", which it said every time.

## [1.7.0] — 2026-10-02

Spend nobody reported no longer reads as zero, and Layover says why a runner's costs are missing
and what to add to its command.

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

- **`layover validate` warns about a runner whose output cannot say what its runs cost**: one an
  agent uses that runs Copilot CLI without `--output-format json` or Claude Code without
  `--output-format stream-json` (or `json`), and a Codex runner without `--json` where a rate card
  has a row for one of its agents' models. Each names the agents affected and what to add. The CLI
  is recognised by name anywhere in the command; a command naming none of them is not judged. A
  factory checked with `validate --strict` that has such a runner now fails the check, which is the
  point: its spend has been unknowable.
- **`layover doctor` says why runs reported no cost, runner by runner.** Beside "7 of 9 run(s)
  reported no cost" it now names each runner the silent runs ran on and what to change: the output
  flag its CLI needs, a rate card row for a Codex model, or — where the command is already right —
  that the runs ended before printing a cost, or were recorded before Layover read one (Copilot
  runs were first priced in 1.4.0, and history is never repriced).

### Fixed

- **A rate card priced nothing.** `[rates.<model>]` is documented as the fallback for a runner that
  prints tokens but no dollars — Codex — and was parsed and validated, and then never applied: every
  such run was `unreported`, and a factory that had written a rate card still showed no spend. A run
  that printed token counts and no dollar figure is now estimated from the card for the model its
  command line selects, with Codex's cached input priced at the cache rate. An estimate is
  `rate_card`: shown and totalled, never debited from Fuel and never drawn from the Reserve, as
  the cost page always said. A figure a runner printed and that was not believed stays
  `unreported`. A run's recorded `model` is now the one its command line selects, falling back to
  the one the agent declares, so a factory that fixes the model in its runner sees it in **By
  model**.
- **Spend that nobody reported read as `$0.00`.** A workflow whose runs all reported nothing showed
  `$0.00` on the route map's **spend, 7d**, with no sign that it was not a figure, and the cost
  cards and breakdowns showed the same sum with only a small note beside it. A total built partly
  from silence now carries a `+`, and one built wholly from it reads **not reported**.
- **`examples/planner.toml` ran Copilot without `--output-format json`**, the only output that
  carries the AI credits a run used, so a factory started from it recorded every Copilot run as
  reporting nothing. Every other example already had it.

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

- **One chain, whole.** Open a chain — from **Chains**, from a run's chain in **Runs**, from the
  buttons under its workflow's map, or straight after triggering it — and see its workflow's map
  drawn for that chain alone: each agent done, running, failed or queued by what happened *in this
  chain*, `×2` on one that ran twice, and the routes its work took drawn in green with the rest
  faded. Below the map, every run in the order it happened with who sent it, how it ended, what it
  took and cost, and **Watch**/**Transcript** and **Report**; then what it has queued. It keeps
  itself current while the chain works, and its address (`#chain=itn_…`) survives a reload.
- **Where each chain is.** Under a workflow's map, one button per chain it has going says where
  that chain is — `46TNEG at coder`, `GDTHVD queued for analyst`. An agent with several runs alive
  at once carries a count, `×2`, on the map. **Chains** says `working · at coder` or
  `working · queued`.
- **Triggering a workflow opens the chain it started**, so what you watch is that run of it.
- **Runs record who sent them.** `sent_by` on a run's record names the agents whose flights started
  it — every arrival, for a released join — and is `[]` for work from outside the mesh. A run it was
  not recorded for says `null` rather than a guess.
- `GET /itineraries/{itinerary_id}`: the chain, its runs oldest first, its queued flights, and its
  map, read at one moment.

### Fixed

- **A chain triggered while every slot was taken was not on Chains** until its first run started:
  chains were built from runs alone. It is listed as soon as it is queued.
- **A workflow's map showed an agent running because another workflow was running it.** The map
  is now coloured by its own workflow's chains, and by runs whose chain no workflow opened, which
  could be anybody's.

## [1.5.1] — 2026-10-01

Dependency and CI maintenance. Nothing a factory sees changes.

**Compatibility.** The configuration format, the HTTP API, the MCP tools and the on-disk layout are
unchanged, and identifiers keep their shape. For library users, `ConfigError::Parse` carries a
`toml` 1.x `toml::de::Error` where it carried a 0.9 one, so code that names that type needs
`toml` 1.

### Changed

- **`ulid` 1.2 → 3.0.** Identifiers are minted with `Ulid::generate`, the new name for
  `Ulid::new`. They are still 26 Crockford base32 characters after their prefix, and
  `RunId::minted_at` still reads the time out of ids an earlier release minted. The dashboard's API
  token still comes from `rand`'s OS-seeded ChaCha generator, now `rand` 0.10.
- **`toml` 0.9 → 1.1.** The TOML 1.1 grammar was already accepted, so what a `layover.toml` may
  contain does not change. `validate --strict`, `explain` and `graph` print the same output as
  1.5.0 for every example, and parse errors read the same.
- **`thiserror` 2.0.20 → 2.0.21.**
- **Workflows.** `actions/checkout` v7, `configure-pages` v6, `deploy-pages` v5,
  `upload-pages-artifact` v5 and `attest-build-provenance` v4. Dependabot no longer updates
  `.github/workflows/release.yml`: `dist` generates it and refuses to release when it differs, so
  its actions move when `cargo-dist-version` does.

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

- **Reply to an agent, and the work continues.** A help request has **Reply** beside **Resolved**.
  It opens with what the agent asked, quoted, to answer between its questions, and sending it
  starts a new run of the agent that asked: a new chain with a fresh budget, from a person, in the
  same workflow with the same flags and routes as the chain that asked — never the workflow's
  defaults. Its work begins `In reply to your help request <run> (<summary>)`, then the answer
  verbatim. The request is marked dealt with, recording who replied, what they said and the chain
  it started, and the two chains name each other. **`POST /help/reply`** does it over the API, with
  the same token as a trigger; it answers `409` for a request already dealt with and `404` for one
  that does not exist.
- **Chains waiting for you.** A chain whose last run stopped on a fatal help request that is still
  open is `awaiting_human` — "waiting for you" on the page, with the request's summary, a
  **Reply…** and an amber count on the Chains tab — instead of `finished`. Resolving the request
  without replying makes it `finished`. `GET /itineraries` takes it as a `state`, counts it as
  `awaiting_human`, and gives each chain `flags`, `waiting_for`, `continues` and `continued_by`.
- **Continue…** on a chain, and in a run's report, opens the trigger window with that chain's
  workflow and flags rather than the defaults, and says where they came from.
- Help requests record the workflow, routes and flags of the chain that raised them, and run records
  the flags they were composed with and the chain theirs continues. `GET /help` and `GET /runs`
  show the flags and the reply.
- The brief every run gets says how a person's answer reaches an agent: as a new run, which is why
  it should write down where it got to before it asks.

- **Sessions: watch agents work, live, the way their CLI would show it.** A new dashboard tab lists
  every run that is going and the ones that ended in the last day. Each opens as a read-only
  terminal: the first lines of the prompt, the model's reasoning, every tool call with a few lines
  of what came back, and what the agent said, with text that is still being written shown as it
  arrives. A finished run replays from the start, so you can see how an agent reached its
  conclusion. **Tile running sessions** shows up to four side by side and adds new ones as they
  start; **Show thinking** and **Follow** do what they say. Runs and reports gain **Watch** /
  **Transcript**. Copilot CLI's JSON events and Claude Code's `stream-json` are rendered;
  anything else is shown as printed. Credentials are masked, and tool output and the prompt are cut
  to their first lines — the whole of both stays in the run's Hangar.
- **`GET /runs/{run_id}/stream`** serves it, as server-sent `entry`, `partial` and `end` events,
  with `?after=` to resume. It answered `501` until now.

### Fixed

- **"Resolved" no longer implies the work will continue.** For a request that stopped its run, the
  tooltip said "If it is not, the next run will raise it again" — but that run ended its chain, and
  there is no next run. It now says resolving restarts nothing, and points to Reply.
- **A run 1.3.0 left behind is settled honestly.** It was recorded as interrupted when the next Tower
  found it — "took 5h47m" for a run that died 29 minutes in — with no workflow and a detail about a
  release not keeping its work. It now ends when its transcript was last written, takes its
  workflow from its chain's other records when any have it, and says plainly that it was lost when
  the Tower stopped and should be re-triggered if still needed. Any run found already gone after a
  restart ends when it was last seen alive.
- **A help request filed while one is being resolved is no longer lost.** Resolving rewrote the
  day's file without holding the lock appends take.
- **A chain whose first run is still going is on the Chains page,** as `working`.
- **The dashboard shows what is running.** It read running runs from history, which records a run
  only when it ends, so nothing ever showed as running: not in **Runs**, not on the route map. Both
  now read the Tower's live records, whichever process started the runs.
- **Empty badges are hidden.** The Chains and Help tabs showed a red "0" when there was nothing to
  count.

### Changed

- `ItineraryState` gains `awaiting_human`. A client that treats it as a closed set will see a value
  it does not know; one that showed every chain that was not `working` as done now shows a chain
  waiting for an answer as done, which is the mistake the state exists to stop.
- For library users: `HelpRequest` gains `scope`, `flags` and `reply`; `RunRecord` gains `flags`
  and `continues`; `Queued` gains `continues`. All are optional on disk, so records written by an
  earlier release still read. `layover_core::help::reply` composes a reply's flight and
  `Journal::answer` queues it and answers the requests as one step.
- For library users: `Ledger` and `Live` are defined in `layover_store::live`, and still exported
  from `layover_tower`. `Ledger::at` reads live records without creating anything, and
  `Ledger::find` looks one up. `layover_store::hangar::run_dir` and `TRANSCRIPT` name a run's
  Hangar and transcript. `Api::stream_run` takes a `StreamRunQuery`.

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

- **Runs overlap: the Tower runs up to `max_concurrent_runs` agents at once.** The setting was
  parsed and read nowhere, and the Tower ran one agent at a time: a sweep that spawned ten
  hour-long reviews took ten hours, manual triggers waited behind it, and a schedule due at 14:00
  fired at 14:23, when the run in front of it ended. The Tower now keeps up to `max_concurrent_runs`
  runs alive, factory-wide, and starts the oldest queued flight that can start the moment a slot
  frees; nothing is refused or dropped. The clock keeps firing while runs are going, and a
  pipeline's tick is still skipped while its last wave is queued *or running*. A fan-out's Hops,
  Fuel and run cap stay exact, `timeout_sec` and a Ground Stop apply to each run, and `layover run`
  runs in parallel too. **An existing factory now runs agents side by side:** two agents that write
  one `work_dir` can write it at the same moment, and runs admitted together can overshoot Fuel or
  the Reserve by up to `max_concurrent_runs` runs' worth. `max_concurrent_runs = 1` restores one at
  a time; `max_concurrent = 1` keeps a single agent from overlapping itself.

- **History files a run by the day it finished.** A run was written into the segment of the day it
  started, while history is read by finish, so an hour-long run crossing midnight was missing from
  its own window.

- **Copilot CLI runs are priced, so Fuel and the Reserve now bind a Copilot factory.** Copilot
  prints no dollars and no token totals, so every Copilot run was `unreported`: `fuel_usd` and the
  Reserve never refused a Copilot factory anything, and `max_runs` and `timeout_sec` were all that
  held. A run is now priced from the last `session.usage_checkpoint` it prints — `totalNanoAiu`
  AI units, in billionths, at `[copilot] usd_per_credit` dollars a credit, by default GitHub's
  published $0.01 — and recorded with a new cost source, `copilot_credits`. It is measured: it
  debits Fuel, draws on the Reserve, counts towards `measured_share`, and is not "reported no
  cost" to `layover doctor`. The dashboard names it: "n of m runs priced from Copilot credits".
  The final `result` event's `premiumRequests` is never priced — it is a flat multiplier per
  prompt. A run killed before its first checkpoint, or whose last checkpoint is negative, not a
  whole number or unreadable, stays `unreported`, and totals built on it stay a lower bound.
  **An existing Copilot factory will now have chains cut by `fuel_usd`**; size it from what runs
  actually cost. Open question 9 in `docs/decisions.md` is decided, and why is in the log.

- **The Reserve is enforced.** It was configured, validated and drawn on the dashboard, and the
  Tower never checked it. Before every run the Tower now adds up the measured spend in history over
  the rolling window and, at `[reserve] fuel_usd`, refuses the run: nothing is spawned, the chain's
  run cap is not charged, and the refusal is recorded as a `halted` run whose detail says how much
  was spent and when the window frees room. `layover doctor` warns about refusals and about a
  Reserve that is exhausted now. **This applies to every factory, including one that writes no
  `[reserve]` table: the documented default of $100 in any rolling 24 hours now binds.** Set
  `fuel_usd = 0` for no ceiling. The Tower reads `layover.toml` once, so a raised cap takes effect
  after a restart.

### Added

- **`[copilot] usd_per_credit`**, what one Copilot AI credit costs, defaulting to `0.01`.
  `layover validate` refuses zero, negative and non-finite values.
- **`copilot_credits`** in the HTTP API's `CostSource`, and **`credit_runs`** on every
  `CostSummary`. A client that treats `CostSource` as a closed set will see a value it does not
  know, and history written by this release carries `"source": "copilot_credits"`, which an earlier
  release cannot read — a downgrade loses those runs from its totals.
- `layover_tower::from_transcript` takes the credit rate as a second argument.
- **`[agents.<name>] max_concurrent`**: at most this many runs of the agent alive at once, within
  `max_concurrent_runs` — `1` for an agent that must never overlap itself, such as a single Teams
  sender. A flight waiting for it does not hold up work for other agents. `layover validate`
  refuses `0`, and warns that a factory-wide `max_concurrent_runs = 0` is read as 1.
- **`GET /flights` says who runs the queue and how busy it is.** `dispatched_by` names the Tower
  under `layover serve` (it was always `null`), and the new `alive_runs` and `max_concurrent_runs`
  give the slots in use. The dashboard shows `2 of 4 run(s) alive · 3 flight(s) queued`, refreshed
  every few seconds, and no longer tells a live Tower's operator that nothing will dispatch their
  work. Under `--watch-only` `dispatched_by` stays `null`, and the page says why.
- **`layover doctor` warns about work that waited with a slot free** — queued longer than
  `timeout_sec` while fewer than `max_concurrent_runs` runs, and fewer than its agent's own cap,
  were alive. That is what the one-at-a-time Tower produced without a word. It checks history and
  the queue as it stands now.
- Run records carry **`queued_at`**, so a run's wait for a slot can be read back.
- For library users: `Factory::dispatch`, `Factory::reconcile` and `layover_tower::recovery`,
  `Factory::alive_runs` and `busy_pipelines`, `Dashboard::dispatched_by`, `layover_store::lock` and
  `Journal::update_learnings`. `Wiring`'s `read_learnings` and `write_learnings` are replaced by one
  atomic `update_learnings`.

### Fixed

- **A run alive when the Tower went away is settled on restart.** It was neither restarted nor
  written to history, and its `state/runs/*.json` stayed behind for good. The next Tower —
  `layover serve`, or `layover run` — now stops it if it is still going (cut off from Layover, it
  could only spend), records it as `interrupted` with what it cost, removes its record, and
  restarts the work where the agent's `recovery` policy and `max_recovery_attempts` allow and no
  Ground Stop is engaged. The restart is told what it is continuing, is charged what the run it
  replaces spent, and — for a join — goes straight to its agent rather than waiting at a barrier
  that no longer exists. Runs another living Tower is watching are left alone.
- **Runs writing at once no longer lose each other's work.** An agent's `memory.md`, the learnings,
  the logbook, help requests, reports and the queue are each changed under a lock, and each
  appended line is written whole. Twelve runs of one agent writing five notes each kept 6 of the 60
  notes before, and learnings writes failed outright on a shared staging file.
- **A rendezvous is not given up while its upstream is still queued**, which a Tower stopping — or
  one with every slot busy — could otherwise do.
- **On Linux, a run that times out or meets a Ground Stop is actually ended.** The Tower asked
  `kill -KILL -<pid>` to end it, which procps 4.0.4 — the `kill` in Ubuntu 24.04 — misreads: it
  signals the process group named by the pid's first digit, and exits 0 when that fails. Nothing
  was killed, so `timeout_sec` and a Ground Stop waited runs out, and a pid beginning with 1 would
  have been `kill(-1)`: every process the user owns. The group is now addressed after `--`, and the
  Tower also ends the run's process itself, without any external command. Windows was not affected.

## [1.3.0] — 2026-09-30

The route map shows each agent's reasoning effort beside its model.

**Compatibility.** Nothing in the configuration format, the HTTP API, the MCP tools or the
on-disk layout changes. Only the drawing does: an agent's first line under its name now reads
`model · effort …` when its command line sets an effort, and a box may have a third small line.

### Added

- **The route map shows each agent's reasoning effort.** It sits beside the model, as
  `claude-opus-5.5 · effort xhigh`, read from the same `--reasoning-effort` flag `GET /agents`
  already reported; it was only in the tooltip. A model name too long to share its line puts the
  effort at the start of the next, and a box grows a third small line rather than cut anything
  off. The tooltip and the pinned agent's panel say "effort" rather than "reasoning", to match.

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

- **The route map says which model each agent runs on.** Under an agent's name is its model, and
  under that its context tier, such as `long context`; hovering shows its reasoning effort, runner,
  access and description. They are read from the command line Layover will run for the agent — its
  runner's `command` with its `model` filled in — so a model fixed in the runner is shown as readily
  as one the agent declares. Only `--model` and Copilot CLI's `--reasoning-effort` and `--context`
  are read. `GET /agents` reports the same as `model`, `reasoning_effort` and `context`; `model` was
  the declared model alone, and was `null` for every agent whose runner fixed it. A factory whose
  command lines name no model is drawn exactly as before.

- **A busy route map is drawn without the tangle.** A pair of plain routes in opposite directions
  is one line with an arrowhead at each end, and a route between two agents in the same column is a
  short connector or arc beside the column, rather than each being a loop under the whole map. A
  join, a spawn or a scope that differs between the two directions keeps its own arrow. A real
  twenty-route workflow went from eleven loops to one.

- **One agent's routes can be traced on the map.** Hovering or focusing an agent lights its routes
  and neighbours and fades the rest; clicking pins them and opens a panel with its model, reasoning
  effort, context tier, runner and access, who it sends to and hears from in that workflow, and a
  link to its runs. Every route has a tooltip saying what it is — `analyst ⇄ sherlock`, or
  `azurix → eagle · spawns a new itinerary · only in eagle-eye` — and a wide invisible edge to
  hover. In the SVG each route is a `<g class="route">` naming its ends in `data-from` and
  `data-to`; a scoped route's tooltip now leads with its ends before `only in …`.

### Fixed

- **A return path ran behind and through the boxes it was routed around.** It dropped straight down
  from the bottom of its source, through every box below it in the same column — showing between
  them as a short vertical line that read as a route between neighbours — and was drawn as one
  curve whose lowest point sits a quarter of the way short of its lane, so a lane just below the
  deepest box put the curve behind that box. It now leaves into the gap right of its column, runs
  down that gap, along its lane and up the gutter, with rounded corners.

## [1.1.0] — 2026-09-29

Workflows that share agents can now keep their routes apart, and nine defects found running a real
factory on Windows are fixed. Three of them stopped a factory doing what its configuration said:
flags chosen at trigger time, a relative `--config`, and an agent's declared MCP servers.

**Compatibility.** Everything new is optional: `pipelines` on a route and on the HTTP `Route`,
`blocker` on `layover_help`, and `layover graph --pipeline`. Queued work and layovers written by
1.1 still read in 1.0.0. Two validation errors are new, each for a factory that could not run as
written: an agent MCP server named `layover`, which would collide with Layover's own; and a
spawned agent that tests a flag its spawning pipeline does not declare, which now inherits that
pipeline's flags instead of borrowing another pipeline's default. One warning is no longer
suppressed: `workspace = "per-itinerary"` isolates nothing yet, so a schedule that can overlap its
own writers is warned about, and `validate --strict` fails on it.

### Added

- **Routes can be scoped to the pipelines whose chains may use them.** `[[routes]]` accepts
  `pipelines = "name"` or a list. A chain may use the global routes and those scoped to its own
  pipeline, and nothing else: `layover_send` along another workflow's edge is refused with the
  usual "call layover_peers" answer, and `layover_peers` lists only what the chain may reach. A
  route without `pipelines` is global, so every existing factory behaves and validates exactly as
  it did. Which pipeline a chain belongs to is the Tower's record, carried on queued work across a
  restart: a spawned chain inherits it, a flight sent straight to an `entry = true` agent belongs to
  none, and a resumed layover belongs to the resuming pipeline but is held to what the chain that
  booked it could reach — so a chain cannot enter another workflow by setting its work down. Joins
  apply only in their scope, and barrier abandonment asks what the barrier's own chain can reach.

  `validate` refuses an unknown pipeline, `pipelines = []`, and overlapping routes that disagree
  about spawning or joins; runs the reach, hop, join and flag checks per pipeline, so a pipeline
  no longer declares flags for agents its routes cannot reach; and warns about a scoped route no
  chain of its scope can use and an `entry = true` agent a direct trigger would strand.

- **Each workflow's route map is drawn over its own routes.** The dashboard's per-workflow
  diagrams, `GET /graph?pipeline=` and the new `layover graph --pipeline` show only the routes that
  workflow's chains may use, so an agent it shares with another workflow appears with only this
  workflow's edges. The whole-factory map labels a scoped edge with its pipelines. `GET /agents`
  gives each route its `pipelines` (`null` when global), and `layover explain` prints each route's
  scope and, once any route is scoped, which agents each pipeline reaches.

### Fixed

- **Flags chosen at trigger time were ignored.** `POST /flights` with
  `{"pipeline":"p","flags":{"flag_x":true}}` was accepted and stored, and the run was composed from
  the pipeline's defaults anyway, so it received `off.md` while `layover prompt --flag flag_x=true`
  showed `on.md`. The stored flags were never read.

  A chain's flags now reach every run it causes. The first run uses what the trigger chose; a
  flight an agent sends carries its chain's pipeline and flags on the queued flight, so they
  survive a Tower restart; a chain opened over a `mode = "spawn"` edge inherits both the flags and
  the pipeline of the chain that spawned it, where it used to merge every pipeline's defaults with
  the last declaration winning; and a layover records the booking chain's flags so its follow-up
  keeps their values for every flag the resuming pipeline declares. A flight to a bare
  `entry = true` agent takes the first declaration of each flag, as `layover prompt` without
  `--pipeline` always did. Prompt-flag validation now follows spawn edges, because a spawned agent
  is composed from the spawning pipeline's declarations.

- **A relative `--config` broke every run.** The factory root stayed relative, so every path a
  child was handed — its Hangar, the `@…/mcp.json` its CLI was pointed at, the `{prompt}` file —
  was relative to the Tower's working directory. The child runs in its agent's `work_dir`, resolved
  them there, and failed in a second: `Failed to read MCP config file …\mcp.json: The system cannot
  find the path specified.` The default `--config layover.toml` with the default
  `work_dir = "workspace"` failed the same way, so a plain `layover serve` could run nothing.

  `serve`, `run` and `autostart` now make the configuration path absolute before anything is
  derived from it, the Tower does the same to the root it is given, and `serve` prints the absolute
  paths it is using. `std::path::absolute` rather than `canonicalize`, whose `\\?\` form many CLIs
  cannot open. An agent's own relative `work_dir` is now resolved against the factory, as the
  configuration reference always said, rather than against wherever the Tower was started.

- **A run record lost the reason a run failed.** A run that died in one second showed
  `exit_code: null` and `detail: null`; the reason was only in its Hangar's `transcript.log`.
  Every run that exits on its own now records its exit code, and a failed one records how it
  exited and the line of its output most likely to be the reason — the last line naming an error,
  or else the last line it printed — redacted and capped at 300 characters.

- **An agent's declared MCP servers never reached it.** `[agents.<name>.mcp.<server>]` validated
  and the book said it worked, but the `mcp.json` a run was handed named only Layover's own server;
  a declared server's only effect was to forward its environment variables. The run's
  configuration now carries every declared server in the runner's dialect — `"type": "stdio"` with
  `command`, `args` and `env`, or `"type": "http"` with `url`, for `claude_json`; one
  `[mcp_servers.<name>]` table each for `codex_toml`, now written to `mcp.toml`. No credential
  value is written: an `env_from` name becomes `"${NAME}"` or `env_vars = ["NAME"]`, read from the
  environment the Tower already gives the child. Layover's own entry gains `"type": "http"`, which
  Claude Code requires for a `url`. `layover validate` refuses an agent server named `layover`.
  Confirmed against Copilot CLI 1.0.88, which listed and called a declared server's tool.

  Two gaps this made visible are recorded as open questions rather than guessed at: `codex exec
  -c` does not accept a file, so Codex MCP wiring cannot work as documented; and an HTTP server
  has no way to receive an `env_from` credential as a header.

- **`layover_help` could not carry the category it asks for.** Every run's brief lists six
  categories and the book documents a `blocker` field, but the tool's schema had no such field and
  the runtime hard-coded `other`, so every request was filed as `other`. `blocker` is now an
  optional enum in the schema; a value that is not a category is refused with a readable
  `isError` naming the ones that exist, and a request without one is `other`. The brief says which
  field carries the category.

  Fixing it exposed that the requests reached nobody at all: they were appended to the agent's
  Hangar, while the dashboard's help tab, `layover doctor` and the run record all read the journal,
  as `book/src/learning.md` said. They now go to the journal, and a run that filed one records its
  summary as `blocked_on`, which had never been set.

- **A woken agent was not told who sent its flight.** The payload held only the body; only a
  released join labelled its senders, although `book/src/prompts.md` and `docs/routing.md` told
  prompt authors to rely on sender identity, so factories hand-rolled a `FROM <agent>` line into
  every body. Every run is now told who sent its flight in a `== WHO SENT THIS ==` section directly
  above the body: an agent by name, a person at the dashboard or API, a pipeline's schedule, or a
  layover coming due. `Origin` gains `Schedule` and `Resumed`, which used to be recorded as
  `Human`. A released join is unchanged. On disk those two are written as `"from":"human"` with an
  optional `via` field, so a 1.0.0 binary still reads — and does not drop — queued work.

- **A resumed layover carried only its `because` text.** `resume_due` built the handover with
  nothing in it, so a follow-up knew only the line the earlier run was waiting for, while
  `book/src/tools.md` and the reference factory's `follower.md` promised it would learn which work
  item this was and what the earlier chain concluded. A layover now records the message that woke
  the run that booked it and which run that was; the resumed run is handed that message and the
  run's last `layover_report`, each quoted and cut to 2,000 characters.

  Reports reached nobody either: `layover_report` appended them to the agent's Hangar while the
  dashboard's report view read the journal. They now go to the journal, where a resumed layover
  and `GET /reports/{run}` find them.

- **`read-only` and per-itinerary workspaces were documented as enforced and are not.** Nothing
  creates a worktree: every agent runs in its `work_dir` whatever its `access` or its pipeline's
  `workspace` says, while the configuration reference called read-only "real enforcement, not an
  advisory flag" and the reference factory's tester and investigator prompts told agents they had
  a snapshot of their own. The documentation, the examples and those prompts now say plainly that
  both are declared and not yet enforced; `layover explain` says so beside the agents and each
  isolated pipeline; and `validate` no longer treats `per-itinerary` as isolation when it warns
  about a schedule that can overlap itself. What an implementation must settle first — including
  that a snapshot at the current commit would not contain a developer's uncommitted change — is
  written up as an open question. `Tool::writes` no longer claims to decide what a read-only agent
  may do; nothing calls it.

## [1.0.0] — 2026-09-23

Layover runs a factory unattended for two days without being touched. That was the bar this
project set for itself, and it has now been cleared rather than asserted.

### The evidence

A soak ran from **2026-09-21 08:47 UTC to 2026-09-23 08:47 UTC** on 0.23.3 — 48.46 hours, one
process, no intervention.

| | |
|---|---|
| Run outcomes | **1,501 / 1,501 succeeded** — none failed, timed out, halted or was interrupted |
| Heartbeat | **1,453** runs on a 2-minute schedule; median gap 120 s, longest 183 s, none over 5 min |
| Real agent chains | **24** two-agent handoffs over MCP, one every two hours, driving the real Copilot CLI |
| Memory after 48 h | **12.6 MB** working set, **106** handles, **19** threads |
| CPU consumed | **1.2 minutes**, total |

The two agents worked against a throwaway repository and independently reached the same
conclusions about it two days apart, which is the behaviour the whole design is for.

### Fixed

- **Hangars were never pruned.** Run history, help requests and the cost ledger all respect the
  ninety-day horizon. The per-run directories holding each run's prompt and transcript did not, so
  a factory grew without bound — and past ninety days it kept transcripts for runs whose records
  had been deleted, which is evidence attached to nothing.

  Found by the soak, which is the argument for having run one: 1,501 runs left 3,050 files behind
  and nothing removed them.

  Pruning reads a run's age from **its own identifier** rather than from the filesystem. A run id
  is a ULID and carries the millisecond it was minted, so the name is exact where `mtime` is a
  guess that a copy, a restore or a backup tool would get wrong.

  Two things are deliberately never pruned: an agent's `memory.md`, which sits beside the run
  directories, and any directory Layover did not mint. The first would silently reset what an
  agent had worked out; the second is somebody else's, and its age is unknown.

### What 1.0 means

Everything in the decision log is built, the reference factory has been proven end to end against
a real agent CLI, all seven crates are on crates.io, and the soak has been run and passed.

It does **not** mean the design is finished. It means the version number stops apologising: from
here a breaking change requires a major bump, and the safety rails — Hops, Fuel, the run cap, the
Reserve, Ground Stop — are a stable contract.

One honest limit, unchanged and documented: **Copilot CLI reports no cost**, only
`premiumRequests`. Fuel and the Reserve cannot bind a Copilot factory; `max_runs` and
`timeout_sec` are what hold. `layover doctor` says so rather than showing a confident `$0.00`.

## [0.23.3] — 2026-09-21

`prompt_file` never reached a run. This is the most serious bug found so far.

### Fixed

- **An agent defined with `prompt_file` ran with no instructions at all.** The Tower composed a
  run's payload from `agent.prompt` — the *inline* form — and never read `prompt_file`. An agent
  that used a file, which is what **all three shipped examples do** and what the book recommends
  because a real prompt composes other files, received a bare `You are \`name\`.` and nothing
  else.

  Everything conspired to hide it. The factory loads. `validate` passes, including
  `validate_prompts`, because the file is real and resolvable. `layover prompt <agent>` renders it
  perfectly, because that command resolves the file properly — so the one tool you would reach for
  to check shows the right answer. And the run *succeeds*: an agent handed a flight body and a
  peer list improvises something plausible, so the output reads like an agent with opinions of its
  own rather than one that was never briefed.

  It surfaced only by reading a real run's composed prompt in its Hangar, during soak preparation,
  after an agent twice declined to do what its prompt file plainly told it to do.

  Prompt **flags** are resolved too, from the pipeline that began the chain — so a prompt whose
  content varies by flag now varies per pipeline, rather than every run silently getting the
  defaults.

- **A prompt that cannot be resolved now refuses the run.** Starting an agent with no instructions
  is the failure above; doing it quietly, after being told exactly which file to use, is worse
  than not starting.

- **The placeholder was emitted twice.** `You are \`name\`.` was both the fallback instructions
  *and* the header the payload composer writes, so a briefing-less agent was told who it was
  twice and nothing else. Identity is the composer's job; the fallback is now empty.

## [0.23.2] — 2026-09-21

### Fixed

- **On Windows, an agent could not run `git`, `npm`, or anything else it shelled out to.** The
  child environment carried `PATH` but not **`PATHEXT`**, and `PATHEXT` is what decides that
  `git` means `git.exe`.

  What hid this for twenty-three releases is that it is invisible to cheap tests. `cmd.exe` falls
  back to a built-in extension list when `PATHEXT` is unset, so every shell stand-in invoked
  through `cmd /c` worked perfectly. **PowerShell has no such fallback** — and PowerShell is what
  agent CLIs shell out through on Windows. The agent reports "`git` is not recognized", which
  reads like a broken machine rather than a stripped environment.

  Found by an agent going off-script during soak preparation: asked only to append a line, it
  tried `git status` first and reported the failure through `layover_report`. The reporting
  channel worked exactly as intended; the thing it reported was ours.

- **A Windows agent CLI could not find credentials it had already been given.** `HOME` was
  forwarded for precisely this reason, but not `USERPROFILE`, `APPDATA` or `LOCALAPPDATA` — the
  Windows spellings of the same idea, and what `~` expands to. An agent that had logged in
  interactively still started as nobody.

  `SystemDrive` joins them for the same class of reason.

  None of these carry a secret; all are facts about the machine, which is the standing bar for
  what the base environment may hold. Credentials still reach a child only through `env_from`.

## [0.23.1] — 2026-09-21

### Fixed

- **The Homebrew formula's `desc` broke `brew style`** at 123 characters against a 118 limit. It
  was a release annotation rather than a failure, but it is also the check homebrew-core applies,
  and there is no reason to carry a warning on every release. Shortened, which improves the
  crates.io listing too.

- **crates.io was a version behind GitHub.** 0.23.0 shipped the Homebrew formula but was never
  published to the registry, so `cargo install layover-cli` still gave 0.22.0.

## [0.23.0] — 2026-09-21

Installable the way people actually install things.

### Added

- **Homebrew.** `brew install KotkaZ/tap/layover` on macOS and Linux.

  A [tap](https://docs.brew.sh/Taps) rather than homebrew-core, because core does not accept
  prebuilt binaries from third parties — it builds everything from source — and applies a
  notability bar this project does not yet meet. A tap costs the user nothing: they are built into
  Homebrew, need no registration, and the `homebrew-` prefix is elided in the install expression.

  The formula is named `layover`, not `layover-cli`, which is what dist would otherwise derive
  from the crate. The crate carries the suffix only because the bare name on crates.io belongs to
  an unrelated SSH tunnelling tool; Homebrew has no such conflict, so the formula gets the name
  the binary actually has.

  Homebrew holds only the *latest* version of a formula. There is no version history, and each
  release replaces what came before.

- **crates.io.** `cargo install layover-cli`, and the seven library crates are published for
  anyone building on them: `layover-core`, `layover-http`, `layover-store`, `layover-mcp`,
  `layover-tower`, `layover-dashboard`.

  Not `cargo install layover`. That name belongs to the SSH tunnelling crate above, whose binary
  is *also* called `layover`, which makes the mistake silent: the install succeeds, the command
  exists, and nothing on your `PATH` is the tool you wanted.

### Fixed

- **The book still said Layover was not on crates.io** and offered only a path install.

## [0.22.0] — 2026-09-20

The reference factory ran end to end on a real agent CLI for the first time. It found three things
that every test had passed over, because every test used a shell stand-in.

A throwaway repository, a planted bug, two agents and the actual `copilot` binary: the Analyst
read the code, found `add` returning `a - b`, handed the finding to the Developer through
`layover_send`, and the Developer fixed it. That works now. It did not before this release.

### Fixed

- **No agent CLI could authenticate.** `env_from` existed only on MCP servers, so an agent's own
  CLI — which needs a credential before it can do anything at all — had no way to receive one.
  The child gets a scrubbed environment by design, so the first real run died with "No
  authentication information found" before it read a word of its instructions.

  `env_from` now exists on an agent and on `[defaults]`, and the two are combined rather than one
  overriding the other: the shared credential every CLI needs goes in `[defaults]`, and a token
  only one agent should hold goes on that agent. Only names appear in the file; the Tower reads
  each value from its own environment at spawn time, and refuses the run when one is unset rather
  than starting a CLI that will fail to authenticate seconds later.

- **Every Copilot example pointed at a flag that does not exist.** The Copilot CLI has no
  `--mcp-config`; it has `--additional-mcp-config`, which takes *either* a JSON string or a file
  path and tells them apart by a leading `@`. Written as it was, MCP was never wired up — so the
  one thing that makes a factory a factory, an agent handing work to another agent, could not
  happen.

  `mcp` wiring now takes an optional `prefix`, and the Copilot examples use
  `{ flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }`.

- **`docs/architecture.md` showed `copilot -p {prompt}` and `claude -p {prompt}`.** `{prompt}` is
  a *path* and `-p` takes the prompt *text*, so a CLI invoked that way is told to go and do
  whatever the string `/path/to/prompt.md` says. The working examples never did this; the
  architecture document did.

- **`layover doctor` hedged about waiting layovers** — "normal, as long as a pipeline declares
  `resumes = true`" — while holding the configuration that answers it. It now checks, and a
  layover waiting in a factory where no pipeline resumes is reported as a **fault**: an agent set
  that work down meaning to come back to it, and there is no way back.

  Found by doctor itself on the first real run, which booked exactly that layover unprompted.

### Documented

- **Copilot CLI does not report cost, and cannot be made to.** Verified against the real binary:
  its `result` event carries `premiumRequests` and durations — no dollars, and no token counts for
  a rate card to work from. So **Fuel cannot bind a Copilot factory**; `max_runs` and
  `timeout_sec` are the rails that actually hold, because they need no cooperation from the
  runner. `layover doctor` surfaces this rather than letting a confident `$0.00` stand.

## [0.21.0] — 2026-09-19

A soak you cannot check is not proof. This adds the command that checks it.

### Added

- **`layover doctor`** reads a factory's recorded history and reports anything a person should
  look at, exiting non-zero when something found would fail an unattended run.

  The bar this project set for itself is forty-eight hours with nobody watching. The problem with
  that bar has been that passing it was a judgement call — you came back two days later, looked at
  a dashboard, and decided. The failures that actually matter are the ones that look like nothing
  from the outside, and a dashboard shows them as nothing:

  - A **stalled chain** reports success on every run it contains. On a list it is indistinguishable
    from a chain that finished.
  - A **cost total built from runners that reported nothing** still renders. It is a floor rather
    than a figure, and the number itself does not say so.
  - A **schedule that never fired** looks exactly like a schedule with nothing to do.
  - A **Ground Stop left engaged** leaves the process up and the dashboard green.
  - An **open help request** is on the one channel that reaches a person, which in a lights-out
    factory is the channel nobody is there to read.

  Findings carry a weight: a *fault* means work was lost or money cannot be accounted for, a
  *warning* means somebody should look, a *note* is worth knowing and does not fail anything. Only
  the first two reach the exit code — a check that failed on every curiosity is one people stop
  running.

  It will not invent a verdict. A factory with no history in the window is reported as having none
  and exits zero: nothing has run, so nothing has passed. In particular "this schedule never fired"
  is not a finding about that schedule when nothing at all has fired, and reporting it per pipeline
  turned a factory nobody had started yet into a page of warnings.

### Fixed

- **`layover-cli`'s README claimed process supervision, the MCP server and the HTTP API were not
  built.** All three have been built for ten releases. It is the page crates.io will show.
- **The CLI reference said "six commands" over a list of seven.**

## [0.20.0] — 2026-09-19

The last of the engineering before 1.0. What remains is proof, not code.

### Security

- **The API requires a token by default.** Minted at startup and printed in the address, so it
  costs one copy-paste; the page keeps it in a `SameSite=Strict`, `HttpOnly` cookie afterwards. It
  is accepted as an `Authorization: Bearer` header, a `?token=` query, or that cookie.

  Loopback alone was a sufficient boundary while this surface only read history. It stopped being
  one when the thing behind it began spending money: anything already on the machine can reach it,
  and so can a page in a browser that knows the port. Such a page cannot *read* a cross-origin
  response, but it can POST one — which here means queueing work a real agent CLI then runs.
  `SameSite=Strict` is the part that closes that.

  The page and its assets are behind the token too. Serving the page and letting its first API
  call fail would look like a broken dashboard rather than a closed door.

  `--no-auth` exists for a machine only you can reach. Binding off loopback *and* passing it now
  warns, because that combination is an open control plane on a network.

### Added

- **Signed build provenance for every artifact**, via `dist`'s own support rather than a hand-edit
  of its generated workflow. A checksum says a file was not altered in transit; it says nothing
  about where the file came from, which is the question that matters when the answer is "a binary
  that will run agent CLIs on your machine".
- **A CycloneDX SBOM attached to each release**, generated from the tag rather than from `main` —
  an SBOM describing a different dependency set from the one shipped is worse than none, because
  it is wrong and looks authoritative. It is attested too: an unsigned claim about supply chain is
  worth little, since anybody can write one.

  Deliberately a separate workflow that runs *after* publication, so it cannot fail a release. An
  SBOM that breaks builds gets switched off within a month.

### Fixed

- **`--no-auth` would have refused every request.** `Option::filter` on a request presenting no
  token yields `None` whatever the guard says, so the open case fell through to the refusal. Found
  by the existing dashboard tests going red as a group.

## [0.19.0] — 2026-09-19

Hardening the two channels that outlive a run. Both were exposures created by earlier releases
rather than found in the abstract.

### Security

- **Learning text is screened before it is stored.** A learning is the most durable foothold in
  this system: it applies to twenty runs with no human watching, sits near the top of a prompt
  where models weight instructions heavily, and its text comes from an agent whose own input may
  have been a work item or a pull request comment. Every other channel an attacker reaches is
  bounded by one run; this one outlives it.

  Refused: text that tries to override instructions, names Layover's own tools, carries a URL, or
  is shaped like a credential. Each refusal says what an acceptable learning looks like, because
  an agent told only "no" re-proposes the same thing next run.

  This became live exposure in 0.18.0, which connected `layover_learn` — and it is a filter on
  obvious attempts, not a guarantee. A patient attacker phrasing an instruction as an observation
  still gets through, and the mitigations for that are the ones already in place: learnings
  expire, an echo cannot confirm one, and a person can drop one.
- **A learning is quoted in the prompt, and flattened onto one line.** Previously inserted raw, so
  text laid out to look like a section heading would have read as prompt structure. The run is
  told why it is quoted: a quoted line telling it to do something is a claim that somebody wrote
  one, and worth reporting rather than following.
- **The prompt sandbox canonicalises.** Lexical confinement handles `..`, absolute paths and UNC,
  and does not handle a symlink *inside* the prompt directory pointing anywhere at all — the path
  is clean, the target is not. The resolved path is now compared against the resolved root. Not
  yet a boundary, because prompt files are reviewed repository content and anyone who can plant a
  symlink can also set `runners.*.command`; it becomes one the moment agents write their own
  prompts, and doing it then would mean doing it under pressure.

### Fixed

- **The credential shape no longer fires on ordinary repository text.** The first version flagged
  any long run of path-ish characters, which caught `tests/data/integration/fixtures`. It now
  looks for what a token actually has and a path does not: a long unbroken run mixing cases *and*
  digits, with `/` and `.` breaking the run. Commit SHAs, paths and shouty filenames pass; a
  filter that fires on normal sentences is one people work around.

## [0.18.0] — 2026-09-19

The factory remembers. All ten tools are connected, and a run is finally given what earlier runs of
it knew.

### Fixed

- **Memory and learnings were never injected.** The supervisor composed every payload with
  `memory: None, brief: ""`, so an agent's own notes and everything earlier runs had worked out
  reached exactly nothing. The machinery on both sides was built and tested — tail-capping,
  decay, rediscovery, Keep and Drop in the dashboard — and the one line joining them was missing.

  This directly contradicted a settled decision: memory is injected rather than fetched precisely
  because an agent that forgets to ask simply has no memory and nothing reports that it forgot. A
  memory system that quietly does not work undoes the decision it was built to serve, which is
  what this was.
- **A learning never aged.** Charging a run against provisional advice is what makes it lapse;
  without it, "applies now and expires unless later runs arrive at it independently" was only the
  first half, and anything proposed once would have applied forever.

### Added

- **`layover_learn` is connected.** A proposal is answered by what became of it, because the
  outcomes are not interchangeable: taken up, an echo of advice the agent was already given
  (evidence of nothing), a genuine rediscovery, or something a human rejected — which repetition
  does not reopen. An agent told "noted" every time learns nothing about what its proposals are
  worth.
- **`layover_logbook_append` is connected**, stamped with who wrote each entry and when. The
  logbook is shared, so an entry nobody can attribute is one nobody can follow up or correct.
- **Runs are charged against provisional learnings whatever the outcome.** A learning that only
  decayed on success would be kept alive by the failures it was meant to prevent.

## [0.17.0] — 2026-09-19

Layover has layovers. The feature the project is named after was a declared tool that answered
"not connected yet" for six releases; it now works.

### Added

- **`layover_wait` sets work down.** An agent that has opened a pull request and wants to react to
  comments over the following days books a layover and *ends*. Nothing stays alive in between — no
  process, no parked chain, no held budget.

  Neither alternative worked. Keeping the chain alive and polling spends a Hop and real money on
  every tick, so Hops kills it long before a human replies — and the whole point of Hops is that it
  should. Re-triggering on a schedule works mechanically but arrives knowing nothing.
- **A `resumes = true` pipeline collects what is due**, on its own schedule. It does not open fresh
  work on its tick; it goes looking for work that was set down. An ordinary pipeline never collects
  layovers, so a factory's hourly sweep cannot quietly start following up somebody else's work.
- **A resumed run opens a new chain with a fresh budget.** The chain that booked the layover is
  over — its Hops and Fuel are spent — and reviving it would make the second follow-up cheaper than
  the first and the tenth refused. A layover is new work about an old subject, and it is priced
  that way. What carries over is context: which chain set this down, what it was waiting for, and
  how many times it has looked.
- **`Cause::Resumed`, which deliberately does not repeat earlier work.** A recovered run may have
  half-applied a side effect and is warned to check. A resumed layover was not interrupted — the
  earlier run finished, having chosen to come back — so it is told the opposite: nothing was left
  half-done. Telling it to look for damage would send it hunting something that was never there.
- **Waits use the same vocabulary as a schedule** — `30m`, `2h`, `3d`. An operator who has written
  `every = "2h"` should not have to learn a second way to say two hours to read a prompt.

### Fixed

- **Booking a layover no longer races the Tower resuming one.** `book` and `amend` are
  read-modify-write over one file, like the queue was, and the Tower now writes there while agents
  do. Both are guarded.

## [0.16.2] — 2026-09-19

**Identical in content to 0.16.0.** Two version numbers were burned recovering from a problem that
did not exist, and the sequence is recorded here rather than tidied away.

0.16.0's release build sat queued behind several unrelated workflow runs. Checking too early, and
trusting a listing that had not caught up, I concluded GitHub had dropped the tag event — it had
not, and that build finished successfully. 0.16.1 was the attempted fix: a `workflow_dispatch`
trigger added by hand to the release workflow. `dist` generates that file and verifies it against
what it would generate, so the edit made `dist host` refuse and the 0.16.1 build fail. The workflow
is back to its generated form.

**0.16.0 and 0.16.2 carry the same code.** 0.16.1 has no release. The lesson is the cheap one: wait
for a queued build before diagnosing it, and do not hand-edit a generated file to fix a fault you
have not confirmed.

## [0.16.1] — 2026-09-19

Tagged; the build failed. No release exists.

## [0.16.0] — 2026-09-19

The factory can ask you things, and now you can answer. The state directory is versioned, so two
releases cannot silently disagree about it.

### Added

- **Help requests can be resolved**, from the API or a button beside each row. Resolving says *the
  blocker is gone*, not *I have read this*: an agent that hits the same wall next run raises it
  again, which is what makes the list evidence of anything. Narrow by run, agent or blocker, or
  clear everything after fixing something that stopped the lot.
- **Learnings can be settled either way.** `PATCH /learnings/{id}` with `confirmed` keeps one
  indefinitely; `rejected` takes it out of every future run.

  **This is not an approval queue**, and the wording throughout says so. A learning applies from
  the moment it is proposed and nothing waits on a human — a sibling project gated learnings behind
  approval and after 22 days held 88 of them, none ever approved, so not one had ever reached a
  run. These are judgements about something already in use.
- **The on-disk layout is versioned.** `.layover/version.json` records which shape the directory
  is. A newer one is **refused** rather than read hopefully: an older build cannot know what it
  does not understand, and writing the directory back without that would turn an afternoon's
  downgrade into permanent loss. An older one is migrated forward once and says so, because a
  silent migration is indistinguishable from a silent corruption until much later.

### Fixed

- **A test that had quietly rotted.** `a_failed_run_colours_its_agent_on_the_route_map` wrote its
  record into a hard-coded day segment while stamping it `now()`. History is one file per UTC day
  and is read by opening the files a span covers, so the record was findable only while the
  calendar stayed within the window — and had just fallen outside it. The helper now derives the
  segment from the record's own timestamp, so the two cannot disagree again.

## [0.15.0] — 2026-09-18

You can now see what the factory is doing and stop it from the page you are watching it on.

### Added

- **The Ground Stop works over HTTP**, and there is a button for it. It answered `501` while
  `layover serve` had become the thing that actually runs agents — so the only way to stop an
  unattended factory was to create a file by hand, at exactly the moment nobody wants to go
  looking for instructions. Engaging twice is a success rather than a conflict: somebody pressing
  again because the first press was not obviously acknowledged must not be told it failed.
- **`GET /itineraries`, and a Chains tab.** A run is one agent doing one thing; a chain is
  everything one trigger caused and the budget it shares. Chains are reported as `working`,
  `finished`, `stalled` or `halted`.
- **`stalled` is real rather than guessed.** When the Tower gives up on a rendezvous it now writes
  the reason to the journal, and the dashboard reads it. Without that record a stalled chain is
  indistinguishable from a finished one — every run in it reports success, and nothing says the
  last step never happened. It is drawn as the loudest thing on the page for the same reason.
- **`DELETE /flights/{id}` cancels queued work.** Only work that has not started: a run already
  going is stopped with a Ground Stop, and saying "cancelled" about something still opening pull
  requests is the most dangerous thing this surface could say.

### Fixed

- **Runs now carry the pipeline that opened their chain.** Every run recorded `pipeline: null`, so
  per-workflow filtering silently matched nothing. The pipeline is remembered per chain rather
  than read off each flight, because only the *first* flight of a chain has one — a flight an
  agent sends carries none, and reading it per flight would label the first hop and lose the rest.

## [0.14.0] — 2026-09-18

The factory runs itself. `layover serve` fires scheduled pipelines, runs what is queued, and serves
agents the endpoint they call back into — so a chain starts, travels and finishes with nobody
watching.

### Added

- **`layover serve` is the Tower.** It was a read-only dashboard; it now also fires schedules,
  drains the queue and hosts the MCP endpoint. `layover autostart` has always registered `serve`,
  which was only useful if `serve` ran the factory. `--watch-only` keeps the old behaviour, for
  looking at a factory another process is running.
- **Schedules fire.** `trigger = { every = "1h" }` and `{ cron = "0 8,18 * * *" }` are evaluated
  against the clock rather than against when the last run finished, so an hourly job does not
  slowly become a ninety-minute one. A Tower that was asleep for six hours fires once on waking,
  not six times.
- **Nothing fires at startup.** A Tower restarting is not a reason to run every hourly job at
  once; if it were, restarting would be expensive enough to avoid.
- **`overlap` on a pipeline.** The default skips a tick whose previous wave is still going —
  starting a second copy means paying twice for one result and, on a shared workspace, two agents
  writing the same files. `overlap = "allow"` opts in. Every skip is reported, because a schedule
  quietly skipping every tick because its work always overruns looks exactly like one that is
  running fine.
- **Barriers hold work while a factory runs.** A flight for a joined agent is parked, and the agent
  wakes **once** when the last declared upstream arrives, with every parked flight and each body
  labelled by who sent it. Two edges into one agent without a join fire it twice; for a publisher
  that is two pull requests for one piece of work.
- **A rendezvous nothing can complete is given up and named**, with what it was holding. Silent
  permanent stalling is the worst outcome in this system: a failure at least says something
  happened.

### Fixed

- **`mode = "spawn"` now does something.** A spawn edge was reported by `layover_peers` and ignored
  by `layover_send`, so a fan-out shared one chain — and a fan-out of twenty pull-request reviews
  would have had the twenty-first refused for a budget the first twenty spent. A spawn now opens a
  fresh itinerary with its own Hops, Fuel and run cap, and does not spend the caller's Hops.
- **The queue is no longer a lost-update race.** `queue` and `unqueue` read the whole queue, change
  it and write it back, which was safe while only one thread did it. Serving the dashboard and
  running the factory in one process made it reachable: a trigger could be accepted and silently
  never happen.
- **`layover validate` no longer warns that a scheduled pipeline "can overlap itself"** when the
  default now stops it doing so. A warning that is not true is one people learn to ignore.

## [0.13.0] — 2026-09-18

Agents reach one another. A run is served an MCP endpoint it can call back into, and a chain is no
longer one hop long.

### Added

- **The MCP endpoint is served, and a run is given a token for it.** `layover run` binds on
  loopback for the length of the drain, writes each run an MCP configuration into its Hangar, and
  passes the address and token in the child's environment. The port is chosen by the operating
  system and read back after binding: a port picked in advance can be taken between picking it and
  binding it, and a child told an address nothing is listening on fails in a way that reads as the
  agent misbehaving.
- **`layover_send` queues a real flight, and the same invocation runs it.** `drain` loops rather
  than iterating once. A run can send while it is running, and draining only the list it started
  with would leave that work sitting until something else happened to pick it up — one hop per
  invocation, forever.
- **A chain shares one itinerary.** Every flight in a causal chain is now accounted against the
  same Hops, Fuel and run cap. Previously `drain` minted a fresh itinerary per flight, which would
  have reset all three on every hop: two agents passing work back and forth would have run forever
  on a budget renewed each time round.
- **The route map is enforced against the live child.** An agent reaching for an edge the map does
  not draw is refused while it runs, and told to call `layover_peers` to see what it can reach —
  rather than finding out after the fact, or not at all.
- **A token dies with its run**, on every path out: a refused plan, a spawn that failed, a timeout,
  a Ground Stop, a clean exit. A token that outlives its run is a finished process that can still
  queue work, with no itinerary to charge it to.
- **An unknown token is refused loudly**, with HTTP 401, before any tool runs. Every other refusal
  in the MCP surface is a successful response the agent can read and act on; this one is not,
  because a call that cannot be accounted to a run must not reach a tool at all.
- **`{mcp}` may be placed in a runner command.** Without it the flag and config path are appended,
  which is what `claude` and `copilot` want. With it they go where the command says — `codex
  exec … -` reads the prompt from stdin and the `-` has to stay last.

### Fixed

- **A drain whose every flight is refused no longer spins.** Only a run can send a flight, so a
  pass that started nothing cannot have produced new work; the loop now stops rather than asking
  a closed queue for more. Found by a test that hung instead of failing.

## [0.12.0] — 2026-09-18

The MCP surface agents talk to Layover through: the protocol, the tool registry, and a check that
stops a prompt naming a tool that does not exist.

### Added

- **`layover-mcp`: JSON-RPC 2.0, `initialize`, `tools/list`, `tools/call`.** Everything here is
  untrusted input — the content that shaped a request came from a work item or another agent — so
  a missing field, a wrong type or an unknown method is answered rather than unwrapped.
- **A tool registry in the domain crate**, which is what lets validation read it. Ten tools, each
  with a description written for the agent that will read it, and deliberately no `layover_spawn`:
  a `mode = "spawn"` route already opens an itinerary per flight, and a tool doing the same would
  be a second permission model over one graph.
- **`layover validate` refuses a prompt that names a tool Layover does not offer.** Eleven names
  were once documented across prompts and the book and none existed, because nothing could compare
  them. An agent told to use a tool it does not have will improvise.
- **Identity comes from the token, never the request.** `Session` is built by the Tower from a
  token it minted; there is no constructor taking an agent name from a request, and a test asserts
  that sending an `agent` field changes nothing.
- **A refused call is a successful response.** "You may not send to that agent" comes back as a
  result marked `isError` with text the agent can act on, not as a JSON-RPC error — which would
  tell the CLI its connection broke rather than telling the agent what it may do instead.
- **A new book page** describing the tool surface and why it is short.

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

[Unreleased]: https://github.com/KotkaZ/layover-project/compare/v1.10.0...HEAD
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
