# Runners, agents and configuration

Part of the [decision log](../decisions.md): MCP servers per agent, credentials, the prompt on stdin, presets, models and effort, and the CLI's home.

**Why MCP servers are declared per agent rather than left to each CLI.** A telemetry agent needs a
Kusto endpoint and a publisher needs an issue tracker, and configuring those in each CLI's own
settings puts them somewhere the route map cannot see, nothing validates, and no reviewer reads.
Declaring them in `layover.toml` makes an agent's reach part of its definition. The cost is that
config now sits next to credentials, which is why `env` and `env_from` are separate: `env` is for
values safe in a committed file, `env_from` names variables forwarded from the Tower's own
environment, and validation *refuses* a literal whose name looks like a credential. A warning
would not do — the failure is a key in git history, which is not undone by noticing later.

**Why a declared server's credential is named in the run's configuration rather than written.**
Declaring servers in `layover.toml` only helps if they reach the run, and the one channel into a
CLI's MCP setup is the configuration file the Tower hands it — which lives in the run's Hangar
under `.layover/`. Writing an `env_from` value there would move the secret from git history to a
directory anything on the machine can read. The Tower already puts the value in the child's
environment, and every supported dialect can say "read it from there": `${NAME}` in the JSON that
Claude Code and Copilot CLI read, `env_vars = ["NAME"]` in Codex's TOML. So the file names the
variable and never holds it. Verified against Copilot CLI 1.0.88, which also passes its whole
environment to the stdio servers it starts; Codex passes only an allow-list, which is why naming
the variable is not optional there.

**Why the prompt is written to stdin and never onto the command line.** The runner template
originally inlined it as `command = ["claude", "-p", "{prompt}", ...]`. Windows caps a command line
at 32,767 characters, and real agent prompts are nowhere near small enough: measured against an
earlier agent factory's prompt trees, its review agent composes to roughly 98 KB — three times over
the limit — and its ordinary developer agent to 34 KB. Two of that project's six agents would not
have started. It is a failure mode that passes every test written against a small fixture and
appears on the first agent worth running, which is the worst shape a bug can have. That project had
already found it the hard way; its runner carries the comment "never `-p` / a shell pipe".
`{prompt}` now substitutes the *path* to the composed instructions, for CLIs that accept one.

**Why the model an agent runs on is read from its command line.** The route map and `GET /agents`
show each agent's model, reasoning effort and context tier. The obvious source was `agent.model`,
and it was wrong for the factories where the question matters most: those that fix the model in
the runner command, one runner per model and permission set, and so declare no `model` at all.
A `context` or `effort` field that *named the flag* was rejected for the reason `{model}` exists
instead of a `model_flag` field — every CLI spells these differently, and the runner command is
already the one place that knows how to invoke a given CLI; a second place would disagree with it
the first time somebody edited one. (The *values* later became agent fields, carried by
placeholders exactly as the model is; see below.) Reading the command line with the agent's values
substituted is also the only honest answer: a declared model whose runner has no `{model}`
placeholder never reaches the CLI.
Only flags whose meaning is certain are read — `--model`, and Copilot CLI's `--reasoning-effort`
and `--context` — because a display that guessed at unknown flags would state a wrong model with
the same confidence as a right one.

**Why an agent declares its effort and context, and the runner only carries them.** With the
values fixed in runner commands, a runner was a model setting *times* a permission set: tuning one
agent's effort meant a new runner and a copied deny list. A real factory did it three times in two
days, ended with two byte-identical runners nobody noticed, and lost two deny rules from every
runner for a week in one of those copy-and-edit rewrites. A runner should say what an agent may do;
how hard it thinks and how much it can read belong to the agent, like its model. So `effort` and
`context` are agent keys carried by `{effort}` and `{context}` the way `{model}` carries a model,
and passed through as written — Copilot CLI refuses an effort a model does not support, and a
catalog in Layover would be a second, staler copy of what only the CLI knows. `[defaults]` gives
both a fallback because a factory usually wants one context tier everywhere; it does not give one
for `model`, which was not asked for and would change what an agent with no model has always run
on.

**Why an unset value takes its flag with it.** Effort and context are optional far more often than
a model, so a placeholder with nothing to fill it had to disappear cleanly. It used not to: a bare
`"--model", "{model}"` with no model left `--model` in front of the next flag, which Copilot CLI
refuses outright, and a joined `--model={model}` reached the CLI as that literal text. The rule now
is one rule for all three: an argument carrying an unset placeholder is left out whole, and when it
is the value of the option before it — the preceding argument starts with `-` and carries no `=` —
the option goes too. Refusing the separate form at validation was the alternative, and was
rejected: it would force every agent on a shared runner to set every value, and it cannot cover
Codex's `-c model_reasoning_effort={effort}`, where the joined form does not exist. The one shape
the pairing can misread — a positional placeholder right after a boolean flag — is not one any
supported CLI uses, and the documentation says to put such a placeholder first. Every factory this
changes was handing its CLI a flag without a value; none of them worked before. A written-out
runner that leaves an agent's effort or context unset is warned about, and a missing model is not, because
factories have long relied on a joined `--model={model}` dropping out and a new warning would fail
their `validate --strict`.

**Why Layover supplies a known CLI's invocation, and only the half that grants.** With effort and
context on the agent, a real factory's five Copilot runners were still eleven identical arguments
and a list of denials each — `--no-ask-user`, `--output-format json`, the three placeholders, the
three `--allow-all-*` — and the identical half was where copy-and-edit had already dropped rules.
Those eleven are not choices: an unattended run cannot answer a question or a permission prompt, a
cost is read from one output only, and the MCP flag is Layover's own wiring. So `cli = "copilot"`
supplies them and `args` carry what is a choice, which for that factory is everything it *denies*.
Granting the `--allow-all-*` by default was decided deliberately (the maintainer, 2026-10-08): an
unattended Copilot run without them stops at the first tool it needs, Copilot's deny rules take
precedence over them, and a runner that wants a narrow allow-list instead writes its `command` out.
Nothing a preset supplies ever takes something away, because restrictions are what has to be written
where it can be reviewed. Claude Code's preset supplies the plumbing and no permission mode: what
`bypassPermissions` grants alongside `--disallowedTools` has not been confirmed against a real
Claude Code, and a preset that guessed wrong would grant more than anybody wrote. Codex has no
preset, because a preset has to wire MCP and how Codex is handed MCP servers is still open (below).
Agents differ from their runner by `args` that are only ever *added*, so a runner stays the
permission floor for every agent on it. An agent that leaves a preset's effort or context unset is
not warned about, as it would be on a written-out `command`: the warning exists for a placeholder
somebody placed meaning it to be filled, and nobody placed a preset's — unset there asks for the
CLI's own default, which is exactly what an agent that says nothing wants.

**Why a length of time has two names rather than a new one.** Schedules were written `"45m"` while
every other length of time was a bare number in a unit its key's name carried, so a four-hour
timeout read `14400` and a reader did the arithmetic (raised by the maintainer, 2026-10-12).
Renaming the keys would break every factory, so each gained a name without its unit — `timeout`,
`window`, `keep_lapsed` — and both names take both forms, a number being read in the original unit.
Naming both is the duplicate-key error TOML would give for any other key, rather than one silently
winning. A key that counts hours or days refuses a duration that is not a whole number of them,
because rounding `"90m"` to one or two hours would change what somebody wrote on purpose.

**Why `http_addr` is honoured and `state_dir` is only warned about.** Both were accepted and did
nothing, and the configuration page said so — a key that is silently ignored is the failure this
project keeps finding, documented or not. `http_addr` had an obvious meaning and nowhere better to
live: the address a factory is served on belongs with the factory, and `autostart` registers `serve`
with no flags, so it was the one way a factory started with the computer could be served anywhere
but 7878. `--addr` still wins, for one start. `state_dir` has no such meaning to give it: moving the
state directory would move history, the queue, Hangars and live-run records together, and a Tower
that read it while an older one did not would split one factory's state in two. So it stays
unhonoured, and `validate` warns where it is set rather than refusing — refusing would stop a
factory that has always loaded.

**Why a separate CLI home is recommended rather than made.** A run keeps `HOME` and `USERPROFILE`
because without them an agent CLI cannot find the sign-in it already has — so by default every run
also writes its session into the user's own CLI home and loads the user's own plugins and MCP
servers. On the maintainer's machine, by 20 October 2026, the factory's separate home held 1,711
sessions and 2.4 GB that would otherwise sit in the list the maintainer resumes from; and a personal
MCP server that posts as its owner is a tool `layover.toml` never granted. That factory already runs
Copilot CLI under `COPILOT_HOME`, set up with only the sign-in copied across, no plugins, and no
global MCP servers. Layover does not create such a home itself: one it made would start signed out,
and how each CLI signs in, where it keeps its token and which of its settings are safe to copy are
the CLI's business, change between its releases, and are not something a supervisor can check. Nor
does a preset set the variable, which would sign every existing factory out at once. So it is a
recommendation, in the configuration reference, the install guide and both starting examples, with
`env_from` as the enforcement: a factory that names the variable refuses a run when it is missing,
rather than falling back to the user's home without saying so.
