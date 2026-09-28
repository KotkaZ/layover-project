# Prompts

An agent's standing instructions can live inline:

```toml
[agents.reviewer]
prompt = "You review work and either approve it or return concrete defects."
```

...or in a file, which is what lets them be composed:

```toml
[agents.tester]
prompt_file = "tester.md"
```

Paths resolve against `[layover] prompt_dir`, which is itself relative to `layover.toml`.

## Conditional includes

A prompt file can pull in others depending on the [flags](./pipelines.md#flags) a run was
triggered with:

```markdown
You are the tester. Run the project's verification command and report a verdict.

@include(run_e2e) tester-e2e.md
@include(!run_e2e) tester-local-only.md
```

Three forms:

| Directive | Meaning |
|---|---|
| `@include path.md` | Always. |
| `@include(flag) path.md` | When `flag` is true. |
| `@include(!flag) path.md` | When `flag` is false. |

A path may be quoted. Included paths resolve **relative to the file that included them**, so a
`roles/tester.md` including `shared.md` gets `roles/shared.md`.

The directive line is *replaced*, not commented out. An agent never sees Layover's own syntax.

```sh
layover prompt tester --pipeline development --flag run_e2e=true
```

That renders exactly what a run would receive, which is how you find out what a conditional
prompt composes to without spending an invocation to see it.

## What you are protected from

Every one of these is an error, not a warning, and every one is a way to silently give an agent
the wrong instructions:

| Problem | Why it is refused |
|---|---|
| A flag the triggering pipeline does not declare | Treating it as false would let a typo delete a whole section. |
| A missing include target | The author believed that text was there. |
| A cycle | Two files including each other. |
| Nesting more than 8 deep | A prompt nobody can reason about. |
| More than 1,000 expansions in one prompt | Shallow includes can still multiply: eight levels of ten files each is millions of reads. The depth cap alone does not bound the total. |
| A path leaving the prompt directory | `../../etc/passwd` is not a prompt. |
| A malformed directive | `@include` with nothing after it. |

`..` is resolved lexically rather than banned outright, so `../shared/common.md` works from a
subdirectory while escaping the root does not.

**Symlinks are followed and checked.** A lexical check sees a clean relative path and lets it
through; only comparing the *resolved* path against the resolved root catches a link inside the
prompt directory pointing outside it. Both checks run: the lexical one refuses the obvious form
before touching the filesystem, and the canonicalising one catches the form that looks innocent.

This is not yet a boundary worth much. Prompt files are repository content under the same review
as the rest of the factory, and anyone who can plant a symlink there can also set
`runners.*.command`, which is arbitrary code by design. It becomes a real boundary the moment
agents write their own prompts — which is a stated goal, and by then it is load-bearing.

## Flags are checked per entry point, not per factory

This is the rule that catches the mistake nobody sees coming.

A run receives the flags of the **one pipeline that triggered it** — never the union of every
pipeline in the factory — with the values chosen when it was triggered. So a prompt is only safe
if every flag it tests is declared by *each* entry point that can reach that agent:

```toml
[pipelines.development]
entry = "analyst"
[pipelines.development.flags]
run_e2e = { default = false }

[pipelines.nightly]          # reaches the same tester...
entry = "analyst"
trigger = { every = "1d" }
                             # ...but declares no flags
```

```text
error: agent `tester` tests flag `run_e2e` in its prompt, but pipeline `nightly` can reach it
       without declaring that flag; the run would fail when the prompt is composed
```

Checking against the union of all flags would have passed that factory, and the nightly run would
have failed at the moment it composed the prompt — hours later, with nobody watching.

"Can reach" includes **spawn edges**. A chain opened over a `mode = "spawn"` route carries the
flags of the chain that spawned it, so a spawned reviewer's prompt is composed from the spawning
pipeline's declarations exactly as a hand-off's would be, and is checked against them.

The same rule applies to a bare `entry = true` agent. It is triggered without a pipeline, so no
flags exist to supply, and any conditional prompt downstream of it is unreachable in practice.
`layover validate` says so.

### The preview is the run

`layover prompt <agent> --pipeline <name> --flag name=value` renders the instructions a run
triggered that way receives, and a run triggered that way — from the dashboard, `POST /flights`, or
a flight its chain sends later — receives exactly that text. See
[a flag holds for the whole chain](./pipelines.md#a-flag-holds-for-the-whole-chain) for where each
run's values come from.

## Writing prompts for fresh runs

Every run is a clean slate. The agent that wakes is not the agent that did the earlier work and
remembers nothing of it, so a prompt has to account for that:

- **Say how the agent can tell why it was woken.** An agent behind a rendezvous receives several
  flights at once; one behind both a join and an ordinary edge can be woken either way. Sender
  identity is how it tells them apart.
- **Say what to write down.** `memory.md` is the agent's entire sense of self across time. A
  scheduled agent that forgets what it already reported will report it again every hour forever.
- **Make optional work explicit rather than conditional.** `join = "all"` waits for every declared
  upstream, so an agent that is asked for help must always reply — *"nothing to add, here is why"*
  is a useful answer and it releases the rendezvous. Silence parks it until the Tower abandons it
  and the itinerary stalls.
- **Say to re-dispatch the whole fan-out on a loop-back.** A barrier resets when any upstream
  delivers twice, so sending a fix to only the agent that complained leaves the barrier waiting
  for a sibling that was never asked.

The last two currently live only in prompts, which is fragile. They are recorded as known risks.
