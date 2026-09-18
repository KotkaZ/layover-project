# The `layover` command

Six commands. `--config` (or `-c`) is global and defaults to `layover.toml` in the working
directory, so it can go before or after the subcommand.

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

## `graph`

```sh
layover graph                          # text
layover graph --svg > factory.svg      # a drawing
```

The route map as a diagram. The SVG is the same renderer the dashboard uses, so it needs no
browser and no JavaScript.

## `prompt`

```sh
layover prompt analyst
layover prompt tester --pipeline development
layover prompt tester --pipeline development --flag run_e2e=true
```

Renders an agent's prompt exactly as a run would receive it, with `@include` directives resolved
and conditional sections resolved against the flags. This is the only way to see what an agent
will actually be told before it costs anything to find out.

## `serve`

```sh
layover serve                          # http://127.0.0.1:7878
layover serve --addr 127.0.0.1:8080
layover serve --history .layover/history
layover serve --watch-only             # dashboard only, start nothing
```

**This is the lights-out command**, and what [`autostart`](#autostart) registers. It does four
things in one process:

| | |
|---|---|
| Fires schedules | A pipeline with a `trigger` starts on its own, and skips a tick whose previous wave has not finished |
| Runs the queue | Whatever is waiting — from a schedule, from `POST /flights`, or sent by another agent |
| Hosts MCP | Every run gets the endpoint and a token, so `layover_send` reaches a real queue |
| Serves the dashboard | The [route map per workflow](./dashboard.md), run history, cost and the Reserve, help requests, learnings, and each agent's report |

They share one process because they share one factory definition, one queue and one set of live
tokens. Splitting them would mean keeping three copies of that agreeing.

It also prunes history past its 90-day horizon on startup.

`--watch-only` leaves out the first three and serves the dashboard alone. That is what you want
when pointing a second window at a factory another process is already running: **two Towers over
one factory directory would race for its queue.**

A Ground Stop is a pause. Engage it and nothing new starts; release it and the next tick fires as
usual.

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

Drains the queue **once** and stops. Each flight is authorised against the route map and the safety
rails, spawned, watched, and written to history; agents can call back over MCP, so a chain sent by
one run is picked up by the same command.

A flight is taken **off** the queue before it runs, so a factory that dies mid-run does not repeat
the work on restart — an agent that opened a pull request and was interrupted before its outcome
was recorded would otherwise open a second one.

A Ground Stop refuses the command outright, and one appearing mid-drain stops it between flights.

**It is not the lights-out command** — that is [`serve`](#serve). `run` is for when you want to
watch one batch of work go through, and for scripting Layover from something else that already has
a scheduler.
