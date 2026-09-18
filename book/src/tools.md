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
| `layover_learn` | Propose something future runs should know. **Not connected yet.** |
| `layover_logbook_append` | Add to the factory's shared memory. **Not connected yet.** |
| `layover_wait` | Set work down to be picked up later. **Not connected yet.** |

The three marked *not connected* are declared and answer honestly when called. Declaring them is
deliberate: a tool that appears and disappears between releases is harder to write a prompt against
than one that says what it is waiting for.

There is deliberately **no `layover_spawn`**. A `mode = "spawn"` route already opens one itinerary
per flight, and a tool doing the same would be a second permission model over the same graph —
two places to look when asking what an agent may start, which is one too many.

## How a run reaches them

`layover run` binds an MCP endpoint on loopback for as long as it is draining, and gives each run
a token minted for it alone. The child is told about both in two ways:

| | |
|---|---|
| `LAYOVER_MCP_URL` | The endpoint, in the child's environment |
| `LAYOVER_RUN_TOKEN` | Its token, in the child's environment |
| `mcp.json` in the run's Hangar | The same two, in the shape the CLI's `--mcp-config` flag expects |

Which file is written depends on the runner's `mcp.format`. The flag is appended to the command
unless the command places `{mcp}` itself:

```toml
[runners.copilot]
command = ["copilot", "--allow-all-tools", "--output-format", "json"]
mcp     = { flag = "--mcp-config", format = "claude_json" }
# runs: copilot --allow-all-tools --output-format json --mcp-config <hangar>/mcp.json

[runners.codex]
command = ["codex", "exec", "--model", "{model}", "{mcp}", "-"]
mcp     = { flag = "-c", format = "codex_toml" }
# runs: codex exec --model <model> -c <hangar>/mcp.toml -
```

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
