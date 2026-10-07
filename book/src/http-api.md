# HTTP API

Layover exposes an HTTP API. The dashboard is purely a client of it, so this list bounds what the
dashboard can ever do.

> **Status:** implemented and served by `layover serve`, which also runs the factory behind it —
> `POST /flights` queues work the Tower then starts. See [Status](./index.md#status).

## The specification is the contract

[`api/openapi.yaml`](https://github.com/KotkaZ/layover-project/blob/main/api/openapi.yaml) is an
OpenAPI 3.2 document, and it is the source of truth rather than a description of one.
`cargo xtask generate-api` turns it into the Rust server — types, an `Api` trait, and the axum
router — and `cargo xtask verify` regenerates it and fails if the result differs.

That means the server cannot drift away from the document's *shape*: an endpoint that exists in
code but not in the specification is impossible, and one that is specified but has no handler is
a compile error rather than a 404 found in production.

It does not mean every handler does what its description says. Generation enforces routes and
types, not behaviour; the tests behind each handler do that.

Point any OpenAPI tool at the file to get a client, a mock server or rendered documentation.

## Endpoints

| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/health` | | Liveness, version, and whether a Ground Stop is engaged. |
| `GET` | `/agents` | | Every agent and the route map between them. Each agent's `model`, `reasoning_effort` and `context` are read from the command line Layover will run for it — with its own `model`, `effort` and `context` filled into its runner's placeholders — and are `null` where it sets none. A route's `pipelines` lists the workflows whose chains may use it, and is `null` for a global route. |
| `GET` | `/pipelines` | | Declared pipelines, their triggers and their flags. |
| `GET` | `/graph` | `pipeline` | The route map as a rendered diagram, optionally for one workflow — drawn over the routes that workflow's chains may use. An agent with several runs alive at once — the workflow triggered twice — carries a count such as `×2`. |
| `POST` | `/flights` | | **Queue** work. The Tower starts it within seconds. Answers with the new chain's `itinerary_id`. |
| `GET` | `/flights` | | What is queued and waiting. |
| `DELETE` | `/flights/{flight_id}` | | Cancel queued work. Only what has not started. |
| `GET` | `/upcoming` | `hours` | What will start on its own. See below. |
| `GET` | `/itineraries` | `window`, `state` | Chains of work, and whether each finished, stalled or is waiting for a person (`awaiting_human`). Each carries its `flags`, `waiting_for` when it waits, the chains it `continues` or is `continued_by`, and where a working chain is: the agents `running` in it and those it has work `queued` for. A chain is listed from the moment its first flight is queued, with no runs yet. |
| `GET` | `/itineraries/{itinerary_id}` | | One chain, whole. See below. |
| `GET` | `/runs` | `status`, `itinerary_id`, `agent`, `pipeline`, `window`, `limit` | Runs, live and historical. Runs alive now come first, as `running`, read from the Tower's live records — history holds a run only once it is over. Each says the `model`, `reasoning_effort` and `context` it ran with, `null` for what its command line did not set and for runs recorded before Layover kept them. |
| `GET` | `/runs/{run_id}` | | One run, including how it ended. |
| `GET` | `/runs/{run_id}/report` | | What that agent wrote about its own run. |
| `GET` | `/costs` | `window`, `pipeline` | What the factory has spent, and how much of it is measured. Each total's `confidence` is the weakest `CostSource` in it — `reported`, `copilot_credits`, `rate_card` or `unreported` — and `credit_runs` counts the runs priced from Copilot AI credits, which `measured_share` counts as measured. |
| `GET` | `/help` | `agent`, `pipeline`, `blocker`, `open`, `window` | Help requests agents have raised. |
| `POST` | `/help/resolve` | | Mark help requests as dealt with. |
| `POST` | `/help/reply` | | Answer a run's help requests and continue the work. See below. |
| `GET` | `/learnings` | `agent`, `state` | Learnings agents have proposed. |
| `PATCH` | `/learnings/{learning_id}` | | Keep a learning for good, or stop using it. |
| `POST` | `/ground-stop` | | Halt everything. Engaging twice is a success, not a conflict. |
| `DELETE` | `/ground-stop` | | Resume. |
| `GET` | `/runs/{run_id}/stream` | `after` | A run's CLI output as server-sent events, rendered as a terminal shows it — live while it runs, a replay once it is over. See below. |

## One chain

`GET /itineraries/{itinerary_id}` answers with everything one trigger caused, read at one moment:

| Field | What it is |
|---|---|
| `itinerary` | The chain, as `GET /itineraries` lists it. |
| `runs` | Every run in it, alive or over, oldest first. Each says who sent it in `sent_by`: the agents whose flights started it — every arrival, for a released join — `[]` for work from outside the mesh (a person, a schedule, a resumed layover), and `null` when that was not recorded. |
| `pending` | Its flights waiting for a slot. |
| `map` | Its workflow's route map drawn for this chain alone: each agent `done`, `running`, `failed` or `queued` by what happened *here*, `×2` on one that ran twice, and the routes its work took marked `travelled`. |

A workflow triggered three times is still one route map, coloured while *any* of its chains runs an
agent; this is how to see one of them. It is read from the moment the chain began — its identifier
carries when — so watching a chain does not read ninety days of history every few seconds. A chain
nothing has run in, is running in or has queued for is `404`.

`null` and `[]` are different on purpose. A run recorded by an earlier release, a join restarted
after the Tower that released it went away, and a run the Reserve refused before it began do not
know who sent them, and a route drawn from a guess would look exactly like one that was taken.

## Watching a run

`GET /runs/{run_id}/stream` follows the run's transcript in its Hangar and sends what is new as
[server-sent events](https://html.spec.whatwg.org/multipage/server-sent-events.html), twice a second
while the run is alive. For a run that is over it sends the whole transcript and closes. It is
read-only: nothing reaches the agent.

```text
id: 48213
event: entry
data: {"at":"2026-10-01T08:02:20.900Z","closes":null,"kind":"tool","text":"view src/lib.rs (1–40)"}

id: 48213
event: partial
data: {"id":"m:5c1e","kind":"say","text":"The change is sound, but"}

id: 51877
event: end
data: {"detail":null,"status":"succeeded"}
```

| Event | Data |
|---|---|
| `entry` | Something the run finished doing. `kind` is `prompt`, `think`, `say`, `tool`, `done`, `failed`, `info` or `raw`; `at` is when the CLI said it happened, or `null`; `closes` names the `partial` it replaces. |
| `partial` | The end of text still arriving. An empty `text` takes it away. |
| `end` | Sent once, last: how the run ended, from history. `"missing": true` when no transcript was kept. |

Every `id` is how far into the transcript the server had read. A client that loses the connection
asks again with `?after=` that `id` and gets only what it has not seen. The output is rendered rather
than raw — deltas folded into the text they build, tool output cut to its first lines, credentials
masked — so a forty-minute run that wrote tens of megabytes arrives as what a person would read.

## Queueing work

`layover serve` prints the dashboard's address with a token in it; send that token with every
request. A Tower started with `--no-auth` needs none.

```sh
curl -X POST localhost:7878/flights \
  -H "authorization: Bearer $LAYOVER_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
        "pipeline": "development",
        "body": "The retry policy drops the last attempt. Fix it.",
        "flags": { "run_e2e": true }
      }'
```

```json
{
  "flight_id": "flt_01JRX...",
  "itinerary_id": "itn_01JRX...",
  "to": "analyst"
}
```

`202 Accepted`, not `200`: the work has been accepted, not finished. The flight is written to the
queue with the pipeline and every resolved flag, so it survives a restart with the run it
describes, and a Tower starts it as soon as a slot is free. `GET /flights` says which Tower —
`dispatched_by` — with `alive_runs` and `max_concurrent_runs`, so a client can tell a busy factory
from a stuck one. `dispatched_by` is `null` when nothing in the serving process runs the queue, as
under `--watch-only`.

Give either a `pipeline` or a `to`. A pipeline is the normal way in — it names the entry agent and
declares which flags may be set. A bare `to` sends to an agent marked `entry = true` and accepts
no flags. Naming an undeclared flag is a `400`, not a silent no-op.

A `409` means a Ground Stop is engaged. A kill switch that halted running work while still
accepting more would not be a kill switch.

## What starts on its own

`GET /upcoming?hours=24` answers what nobody has to trigger, over a window of 1 to 168 hours:

- `workflows` — every scheduled pipeline: `next_at`, whether it is `working` (its last wave still
  queued or running, so its next tick is skipped), `fires_in_window`, and `skipped_7d` with
  `last_skipped_at`.
- `fires` — each tick in the window, soonest first and at most 24 per pipeline. `overdue` marks a
  tick held back, by a Ground Stop usually, that fires as soon as it can; `may_skip` marks the next
  tick of a pipeline still working; `collects` says how many layovers a resuming tick picks up.
- `layovers` — every layover waiting, soonest due first, with `collected_at` and `collected_by`:
  the first tick of a resuming pipeline at or after `due_at`, which is later than `due_at` by up to
  one interval.
- `skips` — ticks skipped in the last seven days, most recent first, with a `reason`.

```json
{
  "now": "2026-10-07T09:54:42Z",
  "until": "2026-10-08T09:54:42Z",
  "clock": "the Tower in `layover serve` (process 9376)",
  "ground_stop": false,
  "workflows": [
    { "pipeline": "follow_up", "next_at": "2026-10-07T10:39:20Z", "resumes": true,
      "overlaps": false, "working": false, "fires_in_window": 32, "skipped_7d": 0 }
  ],
  "fires": [
    { "pipeline": "follow_up", "at": "2026-10-07T10:39:20Z", "overdue": false,
      "resumes": true, "may_skip": false, "collects": 0 }
  ],
  "layovers": [
    { "layover_id": "lay_01M4...", "agent": "publisher", "waiting_for": "comments on pull request 41",
      "booked_by": "itn_01M4...", "pipeline": "development",
      "booked_at": "2026-10-07T09:54:30Z", "due_at": "2026-10-07T11:54:30Z",
      "collected_at": "2026-10-07T12:09:20Z", "collected_by": "follow_up" }
  ],
  "skips": []
}
```

**The times are the Tower's.** An `every` schedule is counted from when the Tower started, so only
the Tower keeping the clock knows when it next fires. A server with no Tower in its process —
`--watch-only` — answers with `clock: null` and no `fires`, rather than a timetable that looks
exact and is wrong by however long ago the real Tower started. Its layovers and skips are still
listed, read from disk.

Work already queued is not here: `GET /flights` lists it, oldest first, which is the order it
starts in.

## Run status

`RunStatus` has two values most APIs would not bother with:

```text
running | succeeded | failed | timed_out | halted | interrupted
```

**`halted` is deliberately distinct from `failed`.** A rail stopping work — Hops, Fuel, the run
cap or the Reserve — is the system doing its job, and colouring it like a crash teaches people to
ignore the colour.

**`interrupted` means the run was alive when the Tower went away.** It is recoverable, and
[recovery](./recovery.md) starts a *new* run rather than resuming this one.

There is deliberately no `stalled`. Stalling is something an **itinerary** does when it parks at a
barrier that can no longer be satisfied; a run either finishes or does not. That state is real and
matters — a factory that quietly parks work forever is worse than one that crashes — but it
belongs to the chain, and `GET /itineraries` carries it.

## Answering a help request

```json
POST /help/reply
{ "run_id": "run_01M3…", "body": "1. Exponential.\n2. The platform team.", "by": "Karl" }
```

`202 Accepted` answers every open request that run filed and queues a flight to the agent that
asked, from a person: a new chain with a fresh budget, in the workflow, with the flags and within
the routes of the chain that asked — never the workflow's defaults. The agent's work begins
`In reply to your help request <run_id> (<summary>)`, then `body` verbatim. The requests are marked
dealt with, recording `by` (or the account the dashboard runs as), the body and the new chain, and
the new chain records the one it continues. The response says which chain, with which flags.

| Status | When |
|---|---|
| `400` | `body` is empty; or `flags` was given for a request that records its chain's, or names one the workflow does not declare |
| `404` | The run filed no help request that is still kept |
| `409` | Every request it filed is already dealt with; or a Ground Stop is engaged; or the request predates Layover recording its chain's flags and `flags` was not given |

A request filed before Layover recorded its chain's flags shows no `flags` in `GET /help`; for one
of those whose workflow declares flags, pass `flags` to say what the chain had. Answering needs the
same token as a trigger.

## Errors

Errors are shaped after RFC 9457:

```json
{ "title": "no such run", "status": 404, "detail": "`run_9` is not a run" }
```

## Security

**A token is minted at startup and printed in the address.** Copy the address, and the page keeps
the token in a `SameSite=Strict` cookie from then on. The token is accepted as an
`Authorization: Bearer` header, a `?token=` query, or that cookie.

Loopback alone was a sufficient boundary while this surface only read history. It stopped being
one when the thing behind it began spending money: anything already on the machine can reach it,
and so can a page in a browser that knows the port. Such a page cannot *read* a cross-origin
response, but it can POST one — which here means queueing work a real agent CLI then runs.

`--no-auth` turns it off, for a machine only you can reach. Binding off loopback *and* passing
`--no-auth` prints a warning, because that combination is an open control plane on a network.

`layover serve --addr` sets the bind address. `[layover] http_addr` is parsed but not yet
honoured.
