# HTTP API

Layover exposes an HTTP API. The dashboard is purely a client of it, so this list bounds what the
dashboard can ever do.

> **Status:** implemented and served by `layover serve`, which also runs the factory behind it —
> `POST /flights` queues work the Tower then starts. Streaming a live run is the one endpoint
> still answering `501`. See [Status](./index.md#status).

## The specification is the contract

[`api/openapi.yaml`](https://github.com/KotkaZ/layover-project/blob/main/api/openapi.yaml) is an
OpenAPI 3.2 document, and it is the source of truth rather than a description of one.
`cargo xtask generate-api` turns it into the Rust server — types, an `Api` trait, and the axum
router — and `cargo xtask verify` regenerates it and fails if the result differs.

That means the server cannot drift away from the document's *shape*: an endpoint that exists in
code but not in the specification is impossible, and one that is specified but has no handler is
a compile error rather than a 404 found in production.

It does not mean every handler does something. Generation enforces routes and types, not
behaviour — which is why one endpoint below compiles, routes, and answers `501`.

Point any OpenAPI tool at the file to get a client, a mock server or rendered documentation.

## Endpoints

| Method | Path | Query | Purpose |
|---|---|---|---|
| `GET` | `/health` | | Liveness, version, and whether a Ground Stop is engaged. |
| `GET` | `/agents` | | Every agent and the route map between them. |
| `GET` | `/pipelines` | | Declared pipelines, their triggers and their flags. |
| `GET` | `/graph` | `pipeline` | The route map as a rendered diagram, optionally for one workflow. |
| `POST` | `/flights` | | **Queue** work. The Tower starts it within seconds. |
| `GET` | `/flights` | | What is queued and waiting. |
| `DELETE` | `/flights/{flight_id}` | | Cancel queued work. Only what has not started. |
| `GET` | `/itineraries` | `window`, `state` | Chains of work, and whether each finished or stalled. |
| `GET` | `/runs` | `status`, `itinerary_id`, `agent`, `pipeline`, `window`, `limit` | Runs, live and historical. |
| `GET` | `/runs/{run_id}` | | One run, including how it ended. |
| `GET` | `/runs/{run_id}/report` | | What that agent wrote about its own run. |
| `GET` | `/costs` | `window`, `pipeline` | What the factory has spent, and how much of it is measured. |
| `GET` | `/help` | `agent`, `pipeline`, `blocker`, `open`, `window` | Help requests agents have raised. |
| `POST` | `/help/resolve` | | Mark help requests as dealt with. |
| `GET` | `/learnings` | `agent`, `state` | Learnings agents have proposed. |
| `PATCH` | `/learnings/{learning_id}` | | Keep a learning for good, or stop using it. |
| `POST` | `/ground-stop` | | Halt everything. Engaging twice is a success, not a conflict. |
| `DELETE` | `/ground-stop` | | Resume. |
| `GET` | `/runs/{run_id}/stream` | | Live output as server-sent events — **`501`**. |

## Queueing work

```sh
curl -X POST localhost:8080/flights \
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

`202 Accepted`, not `200`: the work has been accepted, not finished — and today, not started
either. The flight is written to the queue with the pipeline and every resolved flag, so it
survives a restart with the run it describes. `GET /flights` reports `dispatched_by: null`,
because nothing will pick it up.

Give either a `pipeline` or a `to`. A pipeline is the normal way in — it names the entry agent and
declares which flags may be set. A bare `to` sends to an agent marked `entry = true` and accepts
no flags. Naming an undeclared flag is a `400`, not a silent no-op.

A `409` means a Ground Stop is engaged. A kill switch that halted running work while still
accepting more would not be a kill switch.

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
belongs to the chain, and there is no itinerary endpoint yet to carry it.

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
