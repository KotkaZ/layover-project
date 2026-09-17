# How it works

This page is a map. The design documents themselves live in the repository, beside the code they
describe, so that a change and its rationale land in the same commit.

| Document | What it covers |
|---|---|
| [Architecture](https://github.com/KotkaZ/layover-project/blob/main/docs/architecture.md) | The system design, the locked decisions, and a log of *why* each one was taken. |
| [Routing](https://github.com/KotkaZ/layover-project/blob/main/docs/routing.md) | Route map semantics, fan-out, rendezvous joins, failure paths. |
| [Decisions](https://github.com/KotkaZ/layover-project/blob/main/docs/decisions.md) | Why each choice was made, and the open questions nobody should guess at. |
| [Risks](https://github.com/KotkaZ/layover-project/blob/main/docs/risks.md) | Known hazards and what we intend to do about them. |
| [AGENTS.md](https://github.com/KotkaZ/layover-project/blob/main/AGENTS.md) | The contributor contract, for humans and agents alike. |

## The two ideas worth knowing

### Fresh runs make memory explicit

Nothing carries over implicitly between runs, so an agent's continuity is *exactly what it chose
to write down*. `memory.md` is not a cache — it is the agent's entire sense of self across time.

This is the most opinionated idea in the project. It has consequences: prompts must make agents
deliberate about what they record, and it is why a scheduled agent has to remember what it already
reported or it will report it again every hour forever.

It also resolves re-entrancy for free. Two concurrent runs of one agent share no session state, so
re-entry is safe by construction.

### Identity comes from the Tower, not the agent

A wrapped CLI is a black box that can emit anything. If a child process could say *"I am the
planner and I have seven hops left"*, every safety rail would be advisory.

So the Tower mints a **single-use bearer token per run**. The token — never the agent's claims —
resolves to `(agent_id, itinerary_id)`. Hops and Fuel are held server-side against the itinerary,
and the agent cannot read, forge or refresh them.

This is why the MCP server uses streamable HTTP rather than stdio: one authenticated endpoint
inside the Tower, with each child holding its own token.

## The safety rails

| Rail | Bounds | Held by |
|---|---|---|
| **Hops** | Depth of a chain | The Tower, per itinerary |
| **Fuel** | Total cost | The Tower, per itinerary |
| **Run cap** | Total runs, when cost reporting fails | The Tower, per itinerary |
| **Ground Stop** | Everything | A file on disk, so it survives a crash |

Hops and Fuel are not interchangeable, and the difference is the single most important thing to
understand about sizing a factory. A hop is spent per flight and branches *inherit* the remaining
count rather than splitting it, so Hops bounds depth and says nothing about breadth. Only a shared
per-itinerary budget bounds that.

The run cap exists because Fuel depends on runners voluntarily reporting cost, and not all of them
do. A safety rail that fails silently is worse than no rail, because it is trusted.

## Contributing

One command is the definition of done:

```sh
cargo xtask verify
```

It runs formatting, lints with warnings denied, generated-code freshness, documentation link
checks, the test suite and the doc build. CI runs that exact command and nothing else, and
`rust-toolchain.toml` pins the compiler, so a local pass really is a CI pass.
