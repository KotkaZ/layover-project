# HTTP API

The Tower exposes an HTTP API. The UI is purely a client of it, so this list bounds what the UI
can ever do.

> **Status:** the surface is specified and the server is generated from that specification, but
> nothing is behind it yet. `layover run` does not exist.

## The specification is the contract

[`api/openapi.yaml`](https://github.com/KotkaZ/layover-project/blob/main/api/openapi.yaml) is an
OpenAPI 3.2 document, and it is the source of truth rather than a description of one.
`cargo xtask generate-api` turns it into the Rust server — types, an `Api` trait, and the axum
router — and `cargo xtask verify` regenerates it and fails if the result differs.

That means the server cannot drift away from the document: an endpoint that exists in code but
not in the specification is impossible, and one that exists in the specification but is not
implemented is a compile error rather than a 404 found in production.

Point any OpenAPI tool at the file to get a client, a mock server or rendered documentation.

## Endpoints

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness, version, and whether a Ground Stop is engaged. |
| `GET` | `/agents` | Every agent and the route map between them — the UI's graph view. |
| `GET` | `/pipelines` | Declared pipelines, their triggers and their flags. |
| `POST` | `/flights` | Start work. |
| `GET` | `/runs` | Runs, live and historical. |
| `GET` | `/runs/{run_id}` | One run, including how it ended. |
| `GET` | `/runs/{run_id}/stream` | Live output, as server-sent events. |
| `GET` | `/costs` | What the factory has spent, and how much of it is measured. |
| `POST` | `/ground-stop` | Halt everything. |
| `DELETE` | `/ground-stop` | Resume. |

## Starting work

```sh
curl -X POST localhost:7878/flights \
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

`202 Accepted`, not `200`: the work has been accepted, not finished.

Give either a `pipeline` or a `to`. A pipeline is the normal way in — it names the entry agent and
declares which flags may be set. A bare `to` sends to an agent marked `entry = true` and accepts
no flags. Naming an undeclared flag is a `400`, not a silent no-op.

A `409` means a Ground Stop is engaged.

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

The Tower binds to loopback and is unauthenticated. It can start processes that write to your
filesystem and spend money. **Do not expose it** without putting something in front of it.
