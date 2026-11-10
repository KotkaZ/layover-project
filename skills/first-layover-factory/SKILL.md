---
name: first-layover-factory
description: Set up a user's first Layover factory, the `layover.toml` that says which agents (on GitHub Copilot CLI or Claude Code) do the work, who may hand work to whom, how work starts and how much it may spend. Asks what the factory is for, starts from Layover's tested example for the installed version, adapts it, checks it with `layover validate --strict`, gives it a CLI home of its own, and helps start, watch and stop its first run. Use when asked to create, set up, scaffold or try a Layover factory, an agent factory or a `layover.toml`, or once Layover is installed.
license: Apache-2.0
---

# Set up a first Layover factory

A factory is one file, `layover.toml`, and the directory it sits in. The Tower, `layover serve`,
reads it, starts an agent's CLI whenever work arrives for that agent, and holds each chain of work
to the limits the file sets. Help the user write one that does something they want, check it with
them, and watch it run once.

**Every run is a paid model session, and every agent runs as the user**, with every tool its CLI
has except the ones the file denies. Say both before the first run, and start nothing without the
user's go-ahead.

## 1. Before you start

- `layover about --json` prints the installed version; keep it for step 3. If `layover` is not
  found, install it first: with the `install-layover` skill if you have it, or from
  [the install guide](https://kotkaz.github.io/layover-project/install.html).
- One agent CLI, installed and signed in: GitHub Copilot CLI (`copilot --version`), which Layover is
  proven against end to end, or Claude Code (`claude --version`). Codex cannot reach Layover's
  tools yet, so its agents cannot hand work on; it is not for a first factory.
- On Windows, a runner preset finds only an `.exe`. If `Get-Command copilot` (or `claude`) is a
  `.cmd` or `.ps1` from npm, see
  [Agent CLIs](https://kotkaz.github.io/layover-project/install.html#agent-clis) first.

## 2. Ask what it is for

Ask these together, offering the defaults:

| Question | Default |
|---|---|
| What should the factory do, and what does "done" look like? | None; it is theirs to say |
| Which CLI? | Copilot CLI |
| Where do the agents work? | A new `workspace/` beside `layover.toml`. An existing checkout only if they are happy for agents to change it: on a branch, with nothing uncommitted. |
| How does work start? | By hand, from the dashboard. Or a schedule, as a cron line in local time. |
| How much may one piece of work spend, in runs and in dollars? | `max_runs = 12`, `fuel_usd = 2.00` |
| What must agents never do? | Push to a remote |

Never point `work_dir` at a checkout of Layover's own source.

Use as few agents as the goal needs. One that does the work and one that checks it make a good
first factory; three, as in the example, is plenty. Add no agents, MCP servers or schedules the user
did not ask for.

## 3. Start from the example

Make a new, empty directory for the factory where the user wants it, outside any checkout the agents
will work in. If it already has a `layover.toml`, it is a factory already: stop and ask.

Download Layover's first-factory example,
[`examples/planner.toml`](https://github.com/KotkaZ/layover-project/blob/main/examples/planner.toml),
**at the installed version**, so that it loads with the binary the user has. In the factory's
directory:

```sh
VERSION="$(layover about --json | sed -E 's/.*"version":"([^"]+)".*/\1/')"
curl -fsSL "https://raw.githubusercontent.com/KotkaZ/layover-project/v$VERSION/examples/planner.toml" -o layover.toml
```

```powershell
$version = (layover about --json | ConvertFrom-Json).version
Invoke-WebRequest "https://raw.githubusercontent.com/KotkaZ/layover-project/v$version/examples/planner.toml" -OutFile layover.toml -UseBasicParsing
```

If there is no such tag, because Layover was built from source, use `main` in its place. The file
is commented throughout, and [Your first factory](https://kotkaz.github.io/layover-project/first-factory.html)
explains each part.

## 4. Make it theirs

Keep the file's shape, and change:

- **The agents.** Rename or replace `planner`, `coder` and `reviewer` to fit the goal. Each has a
  `description`, one line that other agents read when choosing whom to send work to; a `purpose`,
  saying when work should come to it; and a `prompt`: what it does, what done looks like, whom it
  sends work to next with `layover_send`, and when it stops instead. A prompt longer than a few
  lines goes in `prompts/<agent>.md`, named by `prompt_file = "<agent>.md"`. See
  [Writing prompts for fresh runs](https://kotkaz.github.io/layover-project/prompts.html#writing-prompts-for-fresh-runs).
  `access = "read-only"` is declared but not yet enforced: every agent can write in `work_dir`.
- **The routes.** Each hand-off is a `[[routes]]` edge, in one direction. An agent with no route
  onward ends the chain. A loop, such as worker to reviewer and back, is how rework happens, and
  `max_hops` and `max_runs` are what bound it.
- **The workflow.** `entry` is the agent that new work arrives at. `trigger = "manual"`, or a
  schedule such as `trigger = { cron = "0 8 * * 1-5" }`.
- **The limits**, in `[defaults]`: `max_hops`, how many hand-offs deep a chain may go; `max_runs`,
  how many runs it may make; `fuel_usd`, what it may spend; `timeout_sec`, how long one run may
  take. Set them from step 2. A scheduled factory should also get a `[reserve]`, one spending limit
  for the whole factory over a window: see
  [Cost](https://kotkaz.github.io/layover-project/cost.html#two-budgets-not-one).
- **The runner.** For Copilot CLI, keep `[runners.copilot]` and add each thing agents must never do
  to its `args`, as a `--deny-tool=…` or `--deny-url=…`; the example already denies
  `shell(git push)`. For Claude Code, replace it, and set `runner = "claude"` in `[defaults]`:

  ```toml
  [runners.claude]
  cli = "claude"
  ```

  Layover grants Claude Code no permission mode, so without one its agents may be refused the
  tools that need permission. Choose one with the user and add it to `args`: `--permission-mode`,
  `--allowedTools`, `--disallowedTools`.
- **The address.** The dashboard listens on `127.0.0.1:7878`. If another factory already uses that
  port, add `http_addr = "127.0.0.1:7880"`, or any free port, under `[layover]`.

Never put an API key or token in the file. Agent CLIs get credentials from the environment, and
`env_from` names the variables they are given. If the user keeps the factory in git, ignore
`.layover/`, which holds its history and every run's transcript.

## 5. Give it a CLI home of its own

Recommend it before the factory runs. Without one, its runs use the user's own CLI home: they fill
the user's session history, and load the user's personal plugins and MCP servers into agents nobody
granted them. If the user agrees:

1. The user signs in once with the variable set, in a terminal of their own, because signing in is
   interactive:

   ```sh
   COPILOT_HOME="$HOME/.copilot-factory" copilot login
   ```

   ```powershell
   $env:COPILOT_HOME = "$HOME\.copilot-factory"; copilot login
   ```

   For Claude Code the variable is `CLAUDE_CONFIG_DIR`, and signing in is `claude` and its `/login`.
2. Uncomment `env_from = ["COPILOT_HOME"]` in `[defaults]`, or name `CLAUDE_CONFIG_DIR`.
3. The variable must be set wherever `layover serve` starts. Offer to make it permanent: a line in
   the shell's profile, or `setx COPILOT_HOME "$HOME\.copilot-factory"` from PowerShell on Windows.
   A Tower started without it refuses each run that needs it and names the variable, rather than
   quietly using the user's own home.

See [a CLI home of the factory's own](https://kotkaz.github.io/layover-project/runners.html#a-cli-home-of-the-factorys-own).

## 6. Check it

In the factory's directory:

```sh
layover validate --strict    # must pass: fix every error and every warning
layover explain              # what can start what, and who may send work to whom
layover prompt planner       # exactly what the entry agent will be told; use its name
```

Show the user what `explain` printed and the entry agent's prompt: that is what they are approving.
`layover graph` draws the route map in Mermaid, if they want to see it.

## 7. The first run

Ask before starting it, and say what one piece of work may spend: up to `max_runs` runs and
`fuel_usd` dollars.

1. **Start the Tower.** `layover serve` runs until it is stopped. It is best started by the user, in
   their own terminal, in the factory's directory, with the CLI-home variable set: they keep it, and
   Ctrl+C stops it. If they ask you to start it, start it in the background so that it outlives your
   command, and tell them how to stop it.
2. **Open the dashboard.** `serve` prints its address with a token in it,
   `http://127.0.0.1:7878/?token=…`. Give the user that address.
3. **Start the work.** On the dashboard the user presses **Trigger…**, picks the workflow and types
   the goal. Start it yourself only when they ask, with
   [`POST /flights`](https://kotkaz.github.io/layover-project/http-api.html#queueing-work).
4. **Watch it.** The chain's page follows the work from agent to agent, and **Sessions** shows each
   agent's CLI live.
5. **Stop it.** Ctrl+C once in the Tower's terminal, or `layover stop` in the factory's directory:
   nothing new starts, what is running finishes, and the Tower exits. Then `layover doctor` reports
   anything a person should look at.

## 8. Report, and what comes next

Tell the user where the factory is; its agents, routes and workflow, in a sentence each; its
limits; whether it has a CLI home of its own; how to start, watch and stop it; and, if it ran, what
the first run did and what it cost.

Then point them on:

- [Starting with the computer](https://kotkaz.github.io/layover-project/install.html#starting-with-the-computer):
  `layover autostart` writes what starts the factory at logon and prints the command that registers
  it. Registering it is theirs to do.
- [Pipelines and triggers](https://kotkaz.github.io/layover-project/pipelines.html) for schedules and
  flags, and [the examples](https://github.com/KotkaZ/layover-project/tree/main/examples) for larger
  factories.
