# Changelog

Notable changes per release. Format follows [Keep a Changelog](https://keepachangelog.com/1.1.0/).

**Pre-1.0: a minor bump may break things.** Crate versions track releases of what is built; the
*first runnable release* — the milestone where a factory actually runs — has not happened yet.
See [project status](README.md#project-status).

## [Unreleased]

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

[Unreleased]: https://github.com/KotkaZ/layover-project/compare/v0.21.0...HEAD
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
