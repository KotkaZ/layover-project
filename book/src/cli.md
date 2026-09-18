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
```

Serves the [dashboard](./dashboard.md) and the [HTTP API](./http-api.md) behind it: the route map
per workflow, run history, cost and the Reserve, help requests, learnings, and each agent's
report. Also prunes history past its 90-day horizon on startup.

This is the most useful command here right now.

## `autostart`

```sh
layover autostart --show               # print it, read it first
layover autostart                      # write it
layover autostart --output ~/svc.xml   # write it somewhere specific
```

Generates your platform's own autostart artefact — a Scheduled Task, a launchd agent or a systemd
user unit — and prints the one command that registers it. It writes nothing if the factory does
not load. See [Install](./install.md#starting-with-the-computer).

## There is no `layover run`

It is deliberately absent rather than stubbed. `run` is the command that will start the Tower, and
while the Tower can now supervise an individual run — spawn a CLI, watch it, end it, price it —
nothing yet routes a message between agents and there is no MCP server for them to talk through.

A command that pretended to start a factory would be worse than one that says it cannot: the
failure would look like a factory with nothing to do. See
[Status](./index.md#status).
