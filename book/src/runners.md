# Runners

A runner is how Layover starts an agent CLI: **a CLI and a permission set**, shared by every
agent with those permissions. Its keys are listed under
[`[runners.*]`](./configuration.md#runners--how-to-invoke-a-cli); this page is what they do.

## What a preset supplies

| `cli` | Layover runs |
|---|---|
| `copilot` | `copilot --model={model} --reasoning-effort={effort} --context={context} --name={name} --allow-all-tools --allow-all-paths --allow-all-urls --no-ask-user --output-format json` then the runner's `args`, the agent's `args`, and `--additional-mcp-config @<hangar>/mcp.json` |
| `claude` | `claude -p --model={model} --output-format stream-json --verbose` then the runner's `args`, the agent's `args`, and `--mcp-config <hangar>/mcp.json` |

That is what an unattended run *requires*: the agent's model, effort and context; nobody to answer
a question (`--no-ask-user`); the one output a cost is read from (`json`, `stream-json` — Claude Code
refuses `stream-json` in print mode without `--verbose`); and Layover's MCP wiring. For Copilot it
also grants what an unattended run could never be asked for — its tools, paths and URLs — so a
runner lists only what it **denies**. Copilot CLI's `--deny-tool` and `--deny-url` rules take
precedence over the `--allow-all-*` flags. A runner that wants a narrower allow-list instead writes
its `command` out.

A model, effort, context or name that is not set is left out whole, so the CLI uses its own
default; on a preset that is not warned about, because nobody wrote those placeholders to be
filled. A run triggered without a name has no `--name`.

A preset starts the CLI by its plain name. On Windows that finds only an `.exe` — WinGet's Copilot
CLI, Claude Code's native build — and not the `.cmd` shim an npm install provides; for that, write
the runner's `command` out naming it, `copilot.cmd`. See [Install](./install.md#agent-clis).

Nothing a preset supplies takes anything away: every restriction an agent runs under is written in
`layover.toml`, where it can be reviewed. Claude Code's permission mode is left to the runner's
`args` — `--permission-mode`, `--allowedTools`, `--disallowedTools` — because what a bypass grants
has not been confirmed against a real Claude Code here.

Repeating something the preset already supplies — a runner migrated from a `command` that kept its
`--no-ask-user` — is warned about, because the CLI would receive it twice; `--model` and its kin in
an agent's `args` are warned about with the key to set instead.

## `command`, written out

```toml
[runners.codex]
command = ["codex", "exec", "--json", "--model", "{model}", "-c", "model_reasoning_effort={effort}",
           "{args}", "{mcp}", "-"]
mcp     = { flag = "-c", format = "codex_toml" }
```

`mcp` says how this CLI is told where Layover's endpoint is: `flag` is the option, `format` the
dialect of the file written into the run's Hangar, and `prefix` anything that must precede the
path. Copilot CLI needs `prefix = "@"` because `--additional-mcp-config` accepts a JSON string
*or* a path and distinguishes them by that character; most CLIs take a plain path and want no
prefix. See [Agent tools](./tools.md). Codex is not given MCP servers yet; see
[decisions](./design/decisions.md#still-open).

## An agent's own arguments

An agent that differs from the rest of its runner by a rule or two says so itself, rather than
needing a runner of its own:

```toml
[agents.builder]
runner = "analysis"
args   = ["--deny-tool=shell(orient reviews:*)"]   # everything `analysis` denies, and this
```

An agent's `args` go where the runner's command has `{args}` — a preset puts it last — or at the
end. A [workflow can add more](./pipelines.md#an-agent-in-one-workflow), and change the agent's
model, effort or context for its own chains. They only ever *add*: nothing an agent or workflow
writes removes what its runner says.

## The output that says what a run cost

Layover reads what a run cost from what its CLI prints, and each CLI prints it in one output mode
only. Leave it out and nothing fails: the work is done, every run is recorded as reporting nothing,
its spend reads **not reported**, and Fuel and the Reserve never bind it.

| CLI | Add | What it then prints |
|---|---|---|
| Copilot CLI | `--output-format json` | The AI credits a run used, which Layover prices |
| Claude Code | `--output-format stream-json` (or `json`) | Dollars |
| Codex | `--json` | Token counts and no price; a [rate card](./cost.md#rate-cards) can estimate them |

`layover validate` warns about a runner an agent uses that runs Copilot CLI or Claude Code without
it, and about a Codex runner without `--json` when a rate card has a row for one of its agents'
models — the cases a change to the command would fix. A `cli` preset supplies it. The CLI is
recognised by name anywhere in a written-out command, so `cmd /c copilot` counts; a command that
names none of them is not judged.

## Telling the model service's failure from the run's

A run the model service ended — a call rejected with `400 internal server error`, retries
exhausted — is restarted like an interrupted one, where its agent's `recovery` allows; a run that
failed its own work never is. Layover tells them apart from the CLI's JSON events, never from text
an agent could have quoted — the one line it reads that is not an event is Copilot CLI's own, whole,
at start-up, saying the model it was given is not available. See
[Recovery](./recovery.md#when-the-model-service-fails).

Copilot CLI is read without being asked: the `copilot` preset, and a `command` that runs it by name
with `--output-format json`. Claude Code and Codex are not read yet. `provider_errors` says
otherwise:

```toml
[runners.wrapped]
command         = ["node", "copilot-wrapper.mjs"]
provider_errors = { read = "copilot" }   # Copilot CLI under another name

[runners.quiet]
cli             = "copilot"
provider_errors = { read = "none" }      # never restart a failed run of this runner

[runners.mine]
command = ["my-cli", "--json"]

[runners.mine.provider_errors]           # a CLI Layover has no reader for
failed     = [{ type = "error", error.kind = "provider" }]
carried_on = [{ type = "tool_call" }, { type = "message" }]
request_id = "error.request_id"
```

| Key | Meaning |
|---|---|
| `read` | `copilot` or `none`. A reader Layover has, instead of `failed`. |
| `failed` | Events that say the model service failed a call. Each is the fields an event must have, with these values; a table matches the object at that field the same way, so `error.kind = "provider"` matches `{"error": {"kind": "provider"}}`. |
| `carried_on` | Events that say the run went on after such a failure: a tool starting, a call that succeeded. |
| `request_id` | Where in a `failed` event the provider's request id is, as a dotted path. Recorded on the run. |

A failed run is the service's when the last event matching `failed` comes after the last one
matching `carried_on`. Only the top-level fields of each line of JSON are matched, so an agent
quoting an error inside a tool result or a message never counts. `validate` refuses a declaration
with both `read` and `failed` or neither, and an empty table, which would match every event; it
warns about `failed` with no `carried_on`, under which a failure the CLI recovered from would still
end the run as the service's.

## Credentials for the CLI itself

An agent CLI needs a credential before it can do anything, and it is not the same credential its
MCP servers need. Name it in `env_from` — under `[defaults]` when every agent uses the same one,
under an agent when only that agent should hold it:

```toml
[defaults]
env_from = ["GH_TOKEN"]          # every agent's CLI can authenticate

[agents.publisher]
env_from = ["RELEASE_TOKEN"]     # and this one alone can publish
```

Only **names** appear here. The Tower reads each value from its own environment when it spawns the
run, so `layover.toml` stays a file you can commit — putting a secret in it is refused at load
time, not discovered in your git history later.

The two lists are combined, not overridden: the publisher above gets both. A name that is not set
in the Tower's environment **refuses the run**, rather than starting a CLI that fails to
authenticate several seconds later and reports it as the agent's failure.

The child otherwise gets a scrubbed environment — `PATH`, `TEMP`, your home directory, and the
handful of variables a process needs to start at all. That is what makes `env_from` meaningful: the
telemetry agent does not hold the publishing token because it never receives it. The home directory
is why a run finds the sign-in you made — and why, unless you
[give it a home of its own](#a-cli-home-of-the-factorys-own), it also writes into yours.

The prompt goes to the process's **stdin**, never onto its command line, and this is not a style
preference. Windows caps a command line at 32,767 characters. Real agent prompts go well past it:
in an earlier agent factory the review agent's prompt tree composes to roughly 98 KB and its ordinary
developer agent to 34 KB. Inlining the prompt passes every test written against a small fixture
and then fails on the first agent worth running.

`{prompt}` is therefore a **path**, not the text — the file the Tower writes the composed
instructions to before spawning. Include it only for CLIs that accept a file of instructions as a
flag; runners without it have the instructions prepended to the stdin payload instead.

## A CLI home of the factory's own

**Recommended for every factory.** Each agent CLI keeps its sign-in, its settings, its plugins and
the history of every session in one directory, and a run started by the Tower finds yours there —
the same `~/.copilot` you use at the keyboard. Two things follow:

- **The factory's sessions bury yours.** Every run is a session, kept like one you started: a
  factory that runs a few hundred times a day fills your session list, and your disk, with
  transcripts you will never resume.
- **Its agents load what you installed for yourself** — your plugins and skills, the MCP servers in
  your own configuration — tools nobody granted them in `layover.toml`, where everything an agent
  may do is meant to be written down and reviewed. A personal MCP server that posts as you is then
  one an unattended agent can call.

Every supported CLI reads its home from one variable:

| CLI | Variable | Default |
|---|---|---|
| Copilot CLI | `COPILOT_HOME` | `~/.copilot` |
| Claude Code | `CLAUDE_CONFIG_DIR` | `~/.claude` |
| Codex | `CODEX_HOME` | `~/.codex` |

Set it where the Tower starts — the script that runs `layover serve`, or the environment of the
service [`autostart`](./install.md#starting-with-the-computer) registers — and pass it on, because a
run receives only the names it is given:

```toml
[defaults]
env_from = ["COPILOT_HOME"]     # e.g. ~/.copilot-factory, set for the Tower
```

Then sign in once with it set — `copilot login`, `codex login`, or `claude` and its `/login` — and
put in it only what the factory needs: no plugins, no global MCP servers, since the ones an agent
needs are [declared for it](./configuration.md#mcp-servers). A CLI that reads a token from the environment, such as
Copilot CLI's `GH_TOKEN`, can be given one through `env_from` instead of a sign-in. Because a name
`env_from` lists that is not set refuses the run — it is recorded as failed, naming the variable — a
Tower started without it stops at its first run rather than quietly writing into your home again.

Layover does not set this itself; the [decision log](./design/decisions/runners.md) says why.

## Placeholders

A runner's command may name these, and each is filled in per run:

| Placeholder | Filled with |
|---|---|
| `{model}` | The agent's `model`. |
| `{effort}` | The agent's `effort`, or `[defaults] effort`. |
| `{context}` | The agent's `context`, or `[defaults] context`. |
| `{name}` | What the run's session is called: the name its chain was given when it was triggered, and the agent. A preset passes it as Copilot CLI's `--name`; left out for a chain nobody named. |
| `{args}` | The agent's own `args`, and any its workflow adds — none, one or several arguments. Optional: without it they are appended at the end. |
| `{prompt}` | The **path** to the composed instructions, for a CLI that takes a file. |
| `{mcp}` | The `mcp` flag and the path to the run's MCP configuration — two arguments. Optional: without it they are appended at the end, which is what `claude` and `copilot` want; `codex exec … -` needs them before its final `-`. |

`{model}`, `{effort}` and `{context}` exist so that **a runner describes a CLI and a permission
set, and nothing about how hard an agent thinks or how much it can read**. A preset carries all
three; a written-out `command` places them where its CLI wants them, since every CLI spells these
flags differently. Either way the value comes from the agent:

```toml
[runners.copilot-analysis]
cli  = "copilot"
args = ["--deny-tool=shell(git push)"]

[agents.eagle]
runner  = "copilot-analysis"
model   = "claude-opus-5.5"
effort  = "xhigh"
context = "long_context"

[agents.tars]
runner  = "copilot-analysis"     # the same permissions, so the same runner
model   = "claude-opus-5.5"
effort  = "high"                 # a different effort needs no second runner
context = "long_context"
```

Codex takes effort as a configuration key, and the placeholder works there too:
`"-c", "model_reasoning_effort={effort}"`.

Values are passed through **exactly as written**. Layover keeps no catalog of models or of the
efforts and context tiers each accepts — Copilot CLI refuses an effort a model does not support,
with a message naming both, and only the CLI knows which those are.

#### A value that is not set

An agent may leave any of the three unset, and then it runs on whatever its CLI defaults to. No
empty argument and no flag without its value ever reaches the CLI — every CLI that takes a value
refuses a flag without one, and some read the next flag as the value instead:

- An argument that **contains** an unset placeholder is left out whole: `--reasoning-effort={effort}`
  disappears entirely, never as `--reasoning-effort=` or as the literal text.
- When that argument is the **value of the option before it** — `"--reasoning-effort", "{effort}"`,
  or Codex's `"-c", "model_reasoning_effort={effort}"` — the option goes with it.

"The option before it" is the argument immediately before, when it starts with `-` and carries no
`=` and no placeholder of its own. That is how every supported CLI pairs a separate value with its
flag, but **prefer the joined form**, `--flag={effort}`: it needs no pairing at all. A CLI that
takes a placeholder positionally right after a boolean flag should put the placeholder first.

This applies to `{model}` too. A separate `"--model", "{model}"` with no model used to leave a
bare `--model` in front of the next flag, and a joined `--model={model}` used to reach the CLI as
that literal text; both now disappear. A factory where every agent sets a model is unchanged.

`layover validate` warns about every way these go wrong quietly:

| Warning | Because |
|---|---|
| An agent sets `effort` or `context` and its runner has no placeholder for it | The value never reaches the CLI. Also said for `model`. |
| A runner whose `command` you wrote has `{effort}` or `{context}` and an agent on it sets none, with no default | The flag is left out, and the CLI's default applies — say so if you mean it. A [preset](#what-a-preset-supplies)'s placeholders are Layover's, there so an agent *may* set a value, so leaving one unset is not warned about. |
| A runner fixes a value beside its placeholder (`--reasoning-effort high` and `{effort}`) | The CLI gets two, and keeps whichever comes last. |
| A `[defaults]` effort or context reaches no runner | Every agent relying on it runs on a runner without the placeholder. |
| An `effort` or `context` is empty | It counts as unset. |

A runner may still **fix** these itself, as every runner had to before the placeholders existed;
every agent on it then runs at the runner's values, and nothing warns:

```toml
[runners.copilot-deep]
command = ["copilot", "--model", "claude-opus-5.5", "--reasoning-effort", "xhigh",
           "--context", "long_context", "--output-format", "json"]
```

#### What is reported

The dashboard, `GET /agents`, the route map, `layover explain` and `layover prompt` report what
each agent runs on by reading its command line **with its own values filled in**, so a value the
runner fixes is reported as readily as one the agent declares, and a value its runner cannot carry
is not reported at all. Only flags whose meaning is certain are read back — `--model`, and Copilot
CLI's `--reasoning-effort` and `--context`, separate or joined. A value carried by a flag this does
not read, such as Codex's `-c`, is reported as the agent declares it. Every run's record keeps the
model, effort and context it ran with; see [Runs](./dashboard-runs.md#runs).
