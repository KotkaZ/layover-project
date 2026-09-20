# The tools an agent has

Layover speaks [MCP](https://modelcontextprotocol.io/), which all three supported CLIs understand
natively. An agent reaches Layover the same way it reaches any other tool server, and the tools
below are what it finds there.

## Why the list is short

Every tool is a thing an agent can do unattended, so each one has to earn its place. The test
applied was whether an agent could do its job without it.

| Tool | What it does |
|---|---|
| `layover_send` | Send work to another agent. **The only way work moves** — and sending is what starts the agent you send to, so there is no separate spawn. |
| `layover_peers` | Who you may send to, and what each is for. Worth calling before deciding where work goes rather than guessing at names. |
| `layover_report` | Say what you concluded. The account of a run that survives it. |
| `layover_help` | Say something is in the way. The channel that stops a quiet failure travelling downstream. |
| `layover_memory_read` | Read your own notes in full. |
| `layover_memory_write` | Add to your own notes, for future runs of you. |
| `layover_status` | What this chain has left: how many messages, how much budget. |
| `layover_learn` | Propose something future runs should know. Applies at once; lapses unless rediscovered. |
| `layover_logbook_append` | Add to the factory's shared memory, stamped with who wrote it. |
| `layover_wait` | Set work down to be picked up later, by a pipeline that resumes layovers. |

**All ten do something.** There is no "declared but not connected" answer left; a tool that
answered honestly about being unfinished was a promise to finish it.

There is deliberately **no `layover_spawn`**. A `mode = "spawn"` route already opens one itinerary
per flight, and a tool doing the same would be a second permission model over the same graph —
two places to look when asking what an agent may start, which is one too many.

## What a run is given

A run is a fresh process that remembers nothing. What it knows comes entirely from its payload,
in this order — instructions, **memory**, **learnings**, handover, and the message that woke it
last, because whatever arrives last reads as the current instruction.

| | |
|---|---|
| **Memory** | The tail of `memory.md` from this agent's Hangar, capped at 4 KB and saying so when it was cut |
| **Learnings** | What earlier runs of *this agent* worked out and that still applies |

Both are **injected, not fetched.** An agent could call `layover_memory_read` when it wants its
notes — cheaper, explicit, and it fails silently: an agent that forgets to call simply has no
memory, and nothing anywhere reports that it forgot. Since fresh runs are what make memory
deliberate in the first place, a memory system that quietly does not work would undo the decision
it was built to serve.

The tail rather than the head because the end of the file is the most recent thing written; a
memory that kept only its oldest entries would get less useful the longer an agent ran. The whole
file stays one tool call away.

### How a learning lives and dies

```text
proposed ──> provisional ──(20 runs, unrediscovered)──> lapsed
                 │                                        │
                 │  rediscovered independently            │
                 └────────────> confirmed <───────────────┘
```

A learning **applies from the moment it is proposed**. There is no approval queue: a sibling
project built one and after 22 days held 88 learnings, none ever approved, so not one had ever
reached a run.

Every run of an agent spends one of its provisional learnings' remaining runs, whatever the
outcome — a learning that only decayed on success would be kept alive by the failures it was meant
to prevent. Run out, and it lapses. Rediscovered independently by a later run, and it counts:
enough times and it becomes permanent.

Repeating advice you were **just given** is an echo, not evidence, and is not counted. Otherwise a
single fluke could confirm itself in three runs.

## How a run reaches them

`layover run` binds an MCP endpoint on loopback for as long as it is draining, and gives each run
a token minted for it alone. The child is told about both in two ways:

| | |
|---|---|
| `LAYOVER_MCP_URL` | The endpoint, in the child's environment |
| `LAYOVER_RUN_TOKEN` | Its token, in the child's environment |
| `mcp.json` in the run's Hangar | The same two, in the shape the CLI's MCP-config flag expects |

Which file is written depends on the runner's `mcp.format`. The flag is appended to the command
unless the command places `{mcp}` itself:

```toml
[runners.copilot]
command = ["copilot", "--allow-all-tools", "--output-format", "json"]
mcp     = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }
# runs: copilot --allow-all-tools --output-format json --additional-mcp-config @<hangar>/mcp.json

[runners.codex]
command = ["codex", "exec", "--model", "{model}", "{mcp}", "-"]
mcp     = { flag = "-c", format = "codex_toml" }
# runs: codex exec --model <model> -c <hangar>/mcp.toml -
```

`prefix` is prepended to the path. Copilot CLI's `--additional-mcp-config` takes *either* a JSON
string or a file path and tells them apart by a leading `@`; without it the path is parsed as JSON
and the run dies complaining about the factory's own configuration. Most CLIs take a plain path
and want no prefix.

`codex exec … -` reads its prompt from stdin, so the `-` has to stay last; that is what `{mcp}` is
for. Everything else can take the append.

### The token is the identity

An agent never says which agent it is. The token does, and Layover holds the mapping — so the
answer to "who is calling?" cannot be influenced by anything in the request, including a work item
or another agent's output that is trying to talk the child into something.

A token is minted as a run starts and revoked the instant its process is gone, on every path out:
a clean exit, a failure, a timeout, a Ground Stop. A call arriving on a revoked token is refused
with HTTP 401 before any tool runs — not as a readable refusal like the others, because a call that
cannot be charged to a run has no chain to spend from and no agent to be.

### What the rails do while a run is live

`layover_send` is checked against the same route map and the same itinerary the supervisor uses:

- An edge the map does not draw is refused, and the agent is told to call `layover_peers`.
- A chain with no Hops left is told to finish and report rather than send, while it can still do
  something about it.
- The flight it queues **continues the caller's chain**. It is not a new itinerary, so it spends
  the same Hops, the same Fuel and the same run cap. Two agents passing work back and forth are
  bounded by the budget the chain started with, not by a fresh one each time round.

## Setting work down

The project is named after this. An agent that has opened a pull request and wants to react to
comments over the following days calls:

```json
{ "until": "6h", "because": "comments on pull request 41" }
```

and then **finishes**. Nothing stays alive in between: no process, no parked chain, no held budget.

Neither alternative worked. Keeping the chain alive and polling spends a Hop and real money on
every tick, so Hops kills it long before a human replies — and the whole point of Hops is that it
should. Re-triggering on a schedule works mechanically but arrives knowing nothing: which work item
is this about, what was already tried, what did the earlier chain conclude.

`until` is how long to wait, in the same vocabulary as a pipeline's `every`: `30m`, `2h`, `3d`. An
agent asked to wait "until the review lands" cannot know when that is, so it names an interval and
is brought back to look.

### Coming back

A pipeline declares that it collects them:

```toml
[pipelines.follow_up]
entry   = "publisher"
trigger = { every = "45m" }
resumes = true
```

A resuming pipeline does not open fresh work on its tick — it goes looking for layovers that are
due. An ordinary pipeline never collects them, so a factory's hourly sweep cannot quietly start
following up somebody else's work.

The resumed run gets a **new chain with a fresh budget**. The chain that booked the layover is over;
its Hops and Fuel are spent, and reviving it would make the second follow-up cheaper than the first
and the tenth refused. A layover is new work about an old subject, and it is priced that way.

What carries over is context. The run is told which chain set this down, what it was waiting for,
when, and how many times it has already looked:

```text
## You are picking up work that was set down

An earlier chain (itn_01M2WH…) finished what it could and chose to come back to this later.
It was waiting for: comments on pull request 41

It was set down at 2026-09-19T09:56:18Z, and this is check 1.

Nothing was left half-done: the earlier run ended cleanly. Your job is to see whether the thing
it was waiting for has happened, and to act on it if it has. If it has not, set the work down
again rather than waiting.
```

That last paragraph is the opposite of what a *recovered* run is told, and deliberately so. A
recovered run may have half-applied a side effect and is warned to check before repeating
anything. A resumed layover was not interrupted — telling it to look for damage would send it
hunting something that was never there.

### When a wait becomes a leak

Each fruitless check pushes the next one further out, doubling from fifteen minutes and capping at
six hours. Backing off is what makes a long wait affordable; the alternative is paying for a run
every few minutes to be told nothing has changed.

After twelve fruitless checks — a little over two days of looking — the layover expires. That is
the difference between waiting patiently and waiting forever, which is the difference between a
follow-up and a leak.

## A prompt cannot name a tool that does not exist

`layover validate` reads every prompt, finds every `layover_*` name in it, and refuses a factory
that tells an agent to call something Layover does not offer:

```text
error: agent `publisher`'s prompt tells it to call `layover_publish`, which is not a tool
       Layover offers; an agent told to use a tool it does not have will improvise
```

This check exists because of a real failure. Eleven tool names were once documented across prompts
and this book, and **none of them existed** — the names drifted apart because nothing could compare
them. Improvising is precisely what a factory is meant not to do unattended.

## Identity comes from the Tower, never from the agent

A tool call carries a token, and the token *is* the identity. Layover looks up which run, which
agent and which itinerary it belongs to; the agent never states any of them.

This is not a formality. Every rail in the system — Hops, Fuel, the run cap, who may send to whom —
is indexed by the agent's name, so an agent that could name itself could claim another agent's
permissions and another agent's budget. There is no code path in which a field an agent sent
becomes an identity, and a test asserts that sending an `agent` field changes nothing.

## A refused call is a successful answer

MCP distinguishes *the call failed* from *the protocol failed*, and Layover uses the distinction.
"You may not send to that agent" is a well-formed answer to a well-formed question, so it comes
back as a result marked `isError`, with text the agent can act on:

```text
`analyst` may not send to `publisher`. Call layover_peers to see who you can reach.
```

Returning that as a protocol error would tell the CLI its connection had broken, rather than
telling the agent it asked for something it is not allowed to have. The agent can read this, and
try something else — which is the entire point of telling it.
