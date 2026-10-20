# Glossary

Layover borrows its words from an airport, and uses them consistently: in `layover.toml`, the
HTTP API, the dashboard and these pages. This is each of them in a sentence, with where to read
more.

| Term | Meaning |
|---|---|
| **Agent** | One configured agent: a name, a runner, a prompt, and what it is for. An airport, in the metaphor. [Configuration](./configuration.md#agents--who-exists) |
| **Flight** | A piece of work in transit to an agent: what one agent, a schedule or a person sends, and what starts the agent it is sent to. [Tools](./tools.md) |
| **Itinerary**, **chain** | Everything one trigger caused: its flights and runs, and the budget they share. The API says itinerary; the dashboard says chain. [Chains](./dashboard-chains.md#chains) |
| **Hops** | How many more flights a chain may take: its depth. Each flight spends one, and a fan-out's branches inherit the count rather than splitting it. [Pipelines](./pipelines.md#sizing-the-rails) |
| **Fuel** | A chain's budget in dollars: its breadth. Every run in the chain draws on it. [Cost](./cost.md) |
| **Run cap** | `max_runs`: how many runs a chain may have, for when a runner does not report what it cost. |
| **Reserve** | The whole factory's budget over a rolling window, checked before every run. [Cost](./cost.md) |
| **Tower** | The supervisor, `layover serve`: it starts runs, holds every counter, and serves the MCP endpoint and the dashboard. |
| **Run** | One supervised execution of an agent CLI. A fresh process that remembers nothing it was not given. |
| **Runner** | How a CLI is started: a CLI and a permission set, shared by every agent with those permissions. A **preset** (`cli = "copilot"`) supplies what an unattended run needs. [Runners](./runners.md) |
| **Route**, **route map** | A directed edge saying one agent may send work to another; the route map is all of them. No route, no flight. [Routes](./routes.md) |
| **Join**, **rendezvous** | A route that parks arriving work until every sender (or any) has delivered, then wakes the agent once with all of it. The parked work waits behind a **barrier**. [Routes](./routes.md#rendezvous-joins) |
| **Spawn** | A route mode that opens a new chain per flight, with its own Hops and Fuel, instead of continuing the sender's. [Routes](./routes.md#spawning) |
| **Pipeline** | How work gets in: an entry agent, a trigger — a person or a clock — and the flags it accepts. [Pipelines](./pipelines.md) |
| **Workflow** | A pipeline seen whole: its trigger, the agents its chains reach and the routes they may use. The dashboard shows one at a time, and each can be switched off. |
| **Flag** | A switch chosen when a workflow is triggered, which prompts test to include or leave out text. It travels with the chain. [Prompts](./prompts.md) |
| **Layover** | Work an agent sets down to be picked up later by a pipeline that resumes it — a pull request waiting for comments, say. [Tools](./tools.md#setting-work-down) |
| **Hangar** | An agent's private state directory: its notes (`memory.md`), and each run's prompt, transcript and MCP configuration. |
| **Logbook** | The factory's shared memory, one Markdown file every agent can add to. |
| **Learning** | Something an agent worked out that later runs of it are told. It applies at once and lapses unless rediscovered. [Learnings](./learning.md) |
| **Help request** | An agent saying something is in its way. A fatal one stops its run and waits for a person to reply from the dashboard. [Help](./learning.md) |
| **Recovery** | Starting a new run on the same flight after a run was interrupted, or ended by the model service, where its agent's `recovery` allows. [Recovery](./recovery.md) |
| **Ground Stop** | The kill switch: running agents are ended and nothing new starts until it is released; parked work is kept. A file on disk, so it survives a crash. [Dashboard](./dashboard.md#stopping-it) |
| **Drain** | Letting what is running finish while nothing new starts — how `layover stop` and an update end a Tower. [Stopping](./cli.md#stop) |
