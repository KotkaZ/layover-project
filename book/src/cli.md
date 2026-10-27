# The `layover` command

Eleven commands. `--config` (or `-c`) is global and defaults to `layover.toml` in the working
directory, so it can go before or after the subcommand.

`serve`, `run` and `autostart` make that path **absolute** before doing anything else, and every
path a run is handed — its Hangar, the `mcp.json` its CLI is pointed at, the `{prompt}` file — is
built from it. A child runs in its agent's `work_dir`, not in the directory the Tower was started
from, so a relative path would be resolved against the wrong folder and the run would fail before
it began. The paths `serve` prints are the absolute ones, so the output names the factory it is
actually running.

```sh
layover --help
layover <command> --help
```

## `validate`

```sh
layover validate                       # layover.toml in this directory
layover validate --config f.toml       # somewhere else
layover validate --strict              # warnings fail too
```

Reports everything wrong with a factory definition and exits non-zero if anything would stop it
starting. Warnings — an agent with no description, a schedule that outruns its own timeout, an
agent beyond the hop budget — are printed but do not fail unless `--strict`.

Worth running in CI over your factory definition. An unattended factory that discovers a typo
three agents deep has already spent money to find out.

## `explain`

```sh
layover explain
```

Describes the factory in prose: its agents, what each one is for, its pipelines and their
triggers, and the route map as a list of edges. The quickest way to check that what you wrote is
what you meant.

Under each agent is what it runs on, read from its runner's command with its own `model`,
`effort` and `context` filled in — so a value its runner cannot carry does not appear:

```text
Agents
  developer [read-write] Implements the work item and repairs what review rejects
      runs on the CLI's default model · effort xhigh · default context
```

A workflow that [runs an agent differently](./pipelines.md#an-agent-in-one-workflow) says so under
the pipeline: `here reviewer runs on claude-opus-5.5 · effort high · long context`.

A route [scoped to workflows](./routes.md#scoping-a-route-to-workflows) carries its scope
on its line, and once any route is scoped each pipeline also says which agents its chains can
reach over the routes they may use:

```text
Pipelines
  eagle-eye [every 7200s] -> azurix
      reaches: azurix, eagle, golddigger, sherlock
...
Routes
  eagle -> azurix, sherlock, golddigger  [pipelines = eagle-eye]
```

A factory with no scoped route prints exactly what it always did.

## `graph`

```sh
layover graph                                # text
layover graph --svg > factory.svg            # a drawing
layover graph --pipeline eagle-eye           # one workflow
```

The route map as a diagram. The SVG is the same renderer and layout the dashboard uses, so it
needs no browser and no JavaScript: columns in reading order, as
[the route map](./dashboard-chains.md#the-route-map) explains. The text is Mermaid source, which
whatever renders it lays out by its own rules.

`--pipeline` draws one workflow over the routes its chains may use — global routes and those
scoped to it — which is the diagram the dashboard shows for that workflow. Without it the whole
factory is drawn, and a scoped edge is labelled with the pipelines that may use it (in SVG, a
`scoped` class and a tooltip).

## `prompt`

```sh
layover prompt analyst
layover prompt tester --pipeline development
layover prompt tester --pipeline development --flag run_e2e=true
```

Renders an agent's prompt exactly as a run would receive it, with `@include` directives resolved
and conditional sections resolved against the flags. This is the only way to see what an agent
will actually be told before it costs anything to find out.

What the agent runs on is printed beside it, on **stderr**, so the prompt itself can still be piped
or diffed — with `--pipeline`, as that workflow runs it:

```text
`eagle` runs on claude-opus-5.5 · effort xhigh · long context, through runner `copilot-analysis`
```

## `serve`

```sh
layover serve                          # [layover] http_addr, http://127.0.0.1:7878 by default
layover serve --addr 127.0.0.1:8080    # somewhere else, this once
layover serve --history .layover/history
layover serve --watch-only             # dashboard only, start nothing
layover serve --no-auth                # open to anything that can reach the port
```

It prints the address with a token in it:

```text
Layover dashboard on http://127.0.0.1:7878/?token=01M2XGEZB8…
The token is in that address; the page keeps it in a cookie afterwards.
```

Copy that once. Loopback alone was a sufficient boundary while this surface only read history; it
stopped being one when the thing behind it began spending money. `--no-auth` turns it off for a
machine only you can reach.

**This is the lights-out command**, and what [`autostart`](#autostart) registers. It does four
things in one process:

| | |
|---|---|
| Fires schedules | A pipeline with a `trigger` starts on its own, on time even while other runs are going, and skips a tick whose previous wave is still queued or running. One [switched off](./dashboard.md#switching-a-workflow-off) lets its ticks pass |
| Runs the queue | Whatever is waiting — from a schedule, from `POST /flights`, or sent by another agent — up to `max_concurrent_runs` at once, the next starting as a slot frees |
| Hosts MCP | Every run gets the endpoint and a token, so `layover_send` reaches a real queue |
| Serves the dashboard | The [route map per workflow](./dashboard.md), run history, cost and the Reserve, help requests, learnings, and each agent's report |

They share one process because they share one factory definition, one queue and one set of live
tokens. Splitting them would mean keeping three copies of that agreeing.

Once a day it also asks github.com whether a newer Layover has been released, and prints a line when
one has — see [`update`](#update). Its factory's `[update] mode = "off"`, or `LAYOVER_UPDATE=off` on
the machine, stops that; a `--watch-only` dashboard never asks.

It also prunes history past its 90-day horizon on startup, and settles what the last Tower left
behind: a run that was alive when that Tower went away is stopped if it is still going — it can no
longer reach Layover — written to history as `interrupted`, and restarted where its agent's
[`recovery`](./recovery.md) policy allows. A run another living Tower is watching is left alone.

`--watch-only` leaves out the first three and serves the dashboard alone. That is what you want
when pointing a second window at a factory another process is already running. **One Tower serves
a factory:** a second `serve` on a factory another is serving refuses to start, because two would
race for its queue.

A Ground Stop is a pause. Engage it and nothing new starts; release it and the next tick fires as
usual.

### Stopping it

**The first Ctrl+C drains.** Nothing new starts — queued work stays queued for the next start, and a
schedule's tick that comes due is held — while what is running finishes and the dashboard and the MCP
endpoint stay up for it. When nothing is alive and no rendezvous holds an arrival, `serve` exits:

```text
  Stopping: nothing new starts. Waiting for 2 run(s) to finish (bob, wolf).
  Press Ctrl+C again, or run `layover stop --now`, to cut it off.
Stopped: everything that was running has finished. Queued work waits for the next start.
```

The one thing a drain still starts is work that completes a rendezvous already holding an arrival —
a flight to the joined agent, or to an upstream it still waits for, in the same chain — because an
arrival parked at a barrier lives only in the Tower's memory and is lost when it stops.

**A second Ctrl+C cuts it off.** Each live run is killed with everything it started, and is left
unrecorded: the next Tower to open the factory records it as `interrupted` and restarts it where its
agent's [`recovery`](./recovery.md) allows, as if this Tower had gone away mid-run. Then the process
exits.

Either way it exits 0, because it stopped as asked: a service manager that restarts a Tower which
fails — the systemd unit and launchd agent [`autostart`](#autostart) writes both do — leaves a
Tower somebody stopped alone.

The dashboard's version pill reads `· stopping` while a Tower drains. A Tower killed from outside —
Task Manager, `kill -9`, a task ended by the scheduler — cuts everything off and leaves its runs
going with nobody listening until the next Tower stops them; use [`stop`](#stop) instead.

## `stop`

```sh
layover stop            # drain, as a Ctrl+C in the Tower's own terminal would
layover stop --now      # cut what is running off, as a second Ctrl+C would
```

Asks the Tower serving the factory to stop, from anywhere: another terminal, a script, the logon
task's own wrapper. It writes `.layover/stop-request.json`, which the Tower looks for twice a second
as it looks for its Ground Stop, and which the Tower removes once it has stopped. No Tower running
means no one to ask: the next one to start ignores the request, and says so. A `--watch-only`
dashboard neither honours the request nor writes one.

## `autostart`

```sh
layover autostart --show               # print it, read it first
layover autostart                      # write it
layover autostart --output ~/svc.xml   # write it somewhere specific
```

Generates your platform's own autostart artefact — a Scheduled Task, a launchd agent or a systemd
user unit — and prints the one command that registers it. It writes nothing if the factory does
not load. See [Install](./install.md#starting-with-the-computer).

## `run`

```sh
layover run                  # run everything queued
layover run --dry-run        # say what would run, start nothing
```

Drains the queue **once** and stops, running up to `max_concurrent_runs` agents at once. Each flight
is authorised against the route map and the safety rails, spawned, watched, and written to history;
agents can call back over MCP, so a chain sent by one run is picked up by the same command. A run
the model service ended is [restarted](./recovery.md#when-the-model-service-fails) after a short
wait where its agent allows, and `run` waits for that restart rather than stopping with it queued:

```text
Ran 2 flight(s):
  wolf failed $0.03 (measured); the model service failed; restarting as attempt 2 after a 1-minute wait
  wolf succeeded $0.02 (measured)
```

Before it reads the queue it settles runs a Tower that went away left behind, as `serve` does —
except with `--dry-run`, which starts nothing and so stops nothing either.

A flight is taken **off** the queue before it runs, so a factory that dies mid-run does not repeat
the work on restart — an agent that opened a pull request and was interrupted before its outcome
was recorded would otherwise open a second one.

A Ground Stop refuses the command outright, and one engaged mid-drain starts nothing more and ends
what is running.

**It is not the lights-out command** — that is [`serve`](#serve). `run` is for when you want to
watch one batch of work go through, and for scripting Layover from something else that already has
a scheduler.

## `doctor`

```sh
layover doctor                      # the last 7 days
layover doctor --window last_24h    # a narrower look
layover doctor --window all_time    # everything still on disk
```

Reads a factory's recorded history and reports anything a person should look at. **Exits non-zero
when something found would fail an unattended run**, which is the point: it turns "did that soak
pass?" into a command rather than a judgement made by squinting at a dashboard two days later.

The failures it looks for are the quiet ones — the ones that look like nothing from the outside:

| Finding | Why it is invisible otherwise |
|---|---|
| A **stalled chain** | Every run in it reports success. A stall and a finished chain look identical on a list |
| Runs reporting no cost | The total reads *not reported* or carries a `+`, which says *that* it is unknown and not *why*. The finding names each runner the silent runs ran on and what to change: the output flag its CLI needs, a rate card row for a Codex model, or — when the command is already right — that the runs ended before printing a cost or predate Layover reading one |
| A schedule that never fired | A schedule that is not firing looks exactly like one with nothing to do. One a person [switched off](./dashboard.md#switching-a-workflow-off) is left out, and named in a note of its own instead |
| Open help requests | The channel that reaches a person is the one nobody is there to read |
| Layovers nothing will collect | Work an agent set down to come back to, in a factory where no pipeline `resumes` |
| A Ground Stop left engaged | The factory is up, the dashboard is green, and nothing is running |
| The Reserve refusing work | A factory that may not spend any more looks exactly like one with nothing to do |
| An agent the model service keeps ending | Each of its runs is restarted, waits and fails the same way, then the attempts run out and the work waits for a person nobody told. Named when its last runs — as many as one piece of work may take, and at least two — were all ended by the model service, with what the CLI said the last time: an outage passes, a model the CLI does not have says `not available` every time |
| Work that waited with a slot free | Queued longer than `timeout_sec` while fewer than `max_concurrent_runs` runs were alive, it looks like a busy factory. It is one where nothing was starting the work. A trigger a switched-off workflow holds is waiting on purpose and is not counted |

Findings come in three weights. A **fault** means work was lost or money cannot be accounted for; a
**warning** means something is wrong and a person should look; a **note** is worth knowing and does
not fail anything. Only the first two affect the exit code — a check that failed on every curiosity
is one people stop running.

Windows are the same set the [dashboard](./dashboard.md) offers: `today`, `last_24h`, `last_7d`,
`last_30d`, `last_90d`, `month_to_date`, `all_time`.

Its first line says which Layover is running and what this machine last learned about releases —
`Layover 1.14.0; 1.15.0 is available (minor): layover update check says what changes. Checked 3
hour(s) ago.` — read from that record, without asking GitHub again. It is not a finding: a release
not yet installed fails nothing.

### It will not invent a verdict

A factory with no history in the window is reported as exactly that, and exits **zero**. Nothing
has run, so nothing has passed and nothing has failed — and a schedule that has not fired is not a
finding about that schedule when *nothing at all* has fired. Widen `--window` if you expected
history and see none.

## `update`

```sh
layover update check              # look now, and say what a newer release changes
layover update stage              # get it ready to install: downloaded, verified, checked here
layover update apply              # install it: the Tower drains, swaps and restarts on it
layover update apply --now        # cut what is running off rather than waiting
layover update rollback           # back to the version the last update replaced
layover update status             # where things stand, asking nothing
layover update log                # what was installed and rolled back here
layover update skip 1.15.0        # stop offering 1.15.0 on this machine
layover update skip --clear       # offer every skipped version again
```

`check` asks for the newest release on the factory's [`[update] channel`](./configuration.md#update--hearing-about-a-newer-layover)
— `stable` when there is no factory — whatever the schedule, and says what it found:

```text
Layover 1.14.0 (x86_64-pc-windows-msvc), stable channel.
1.15.0 is available — released 2026-10-17, minor.
  **Compatibility.** Additive for `layover.toml`: every factory that loaded under 1.14 loads unchanged…
Release notes: https://github.com/KotkaZ/layover-project/releases/tag/v1.15.0
`layover update apply` installs it; a Tower serving the factory lets what it is running finish first.
```

The indented paragraph is the release's own Compatibility note: what an existing factory will
notice. It exits non-zero only when it could not look — offline, refused by a proxy, or
`LAYOVER_UPDATE=off`, under which it makes no request at all.

A Tower whose factory has `[update] mode = "notify"`, the default, does the same once a day by itself
and prints one line when it finds something; the [dashboard](./dashboard.md#a-newer-layover) and
[`doctor`](#doctor) say so too. Nothing is downloaded and nothing changes: installing a release is
still yours to do, the way you [installed](./install.md) Layover.

`stage` goes further and gets the newest release ready, without installing it — every step is in
[staging a release](./updating.md#staging-a-release):

```text
Layover 1.15.0 for x86_64-pc-windows-msvc:
  layover-cli-x86_64-pc-windows-msvc.zip: SHA-256 1842a1c7… matches the release.
  Provenance verified: built by https://github.com/KotkaZ/layover-project/.github/workflows/release.yml@refs/tags/v1.15.0.
  It says it is 1.15.0 for x86_64-pc-windows-msvc, state layout 1, the same as this one.
  It validates C:\factory\layover.toml, with no new warnings. Ready to install.
```

Verifying needs [`gh`](https://cli.github.com) installed and signed in. Without it the release is
downloaded and checked against its SHA-256, and not run; `--trust-checksum-only` runs it anyway, on
your say-so. It exits non-zero when the release is refused, or refuses the factory. A Tower whose
factory has `[update] mode = "stage"` stages each new release after its daily look, and prints what
it found.

`apply` installs the newest release, staging it first if it is not, once it is verified and
validates the factory — the whole of it is in
[installing a release](./updating.md#installing-a-release). With a Tower serving the factory
it asks that Tower and follows it:

```text
The Tower serving this factory will drain and install 1.15.0. Ctrl+C here stops watching, not the install.
  Installing 1.15.0: nothing new starts until it is in.
  Waiting for 2 run(s) to finish (bob, wolf).
  Nothing is running or parked.
  Installed 1.15.0; 1.14.0 is kept at Q:\.tools\.cargo\bin\layover-1.14.0.exe.old.
  Started 1.15.0 (pid 24220); waiting for it to answer on 127.0.0.1:7879.
  Updated 1.14.0 → 1.15.0. The new Tower writes to C:\factory\.layover\serve.log.
```

`--now` cuts what is running off rather than waiting `[update] max_drain` for it; an agent whose
`recovery` is `manual` still postpones the install, unless `--including-manual` is given too.
`--trust-checksum-only` installs a release whose provenance could not be checked. With no Tower
serving the factory, the binary is replaced at once.

`rollback` goes back to the version the last update replaced, while it is kept, and stops offering
the one it leaves. `status` says where things stand — the version and how it was installed, what
is staged, an install under way, the last update and whether it proved healthy — from what is on
disk, asking nothing. `log` prints `.layover/updates.jsonl`.

A factory with `[update] mode = "auto"` does what `apply` does by itself, within the limits in
[installing unattended](./updating.md#installing-unattended).

`apply` and `rollback` say how it went in their exit code:

| Code | Meaning |
|---|---|
| `0` | Installed, or already the newest |
| `2` | Nothing to install, or nothing kept to go back to |
| `3` | Refused: not verified, refuses the factory, changes the state layout, installed by a package manager, or failed and rolled back by the Tower |
| `4` | Postponed: a `manual` agent was still running at the deadline |
| `5` | Rolled back |

`skip` is for a release you have decided against. It is kept per machine, like what was found, and
a newer release is offered as usual.

## `about`

```sh
layover about           # what this binary is
layover about --json    # the same, for a program
```

```json
{"version":"1.15.0","target":"x86_64-pc-windows-msvc","layout":1,"api":"1.15.0","min_upgrade_from":"1.0.0"}
```

The version; the platform it was built for; the version of the
[state directory's layout](./configuration.md#the-state-directory-is-versioned) it reads and writes;
the HTTP API it serves; and the oldest release it can be updated from in one step. An updater asks
a binary this before trusting it, which is why every release from now on carries it.
