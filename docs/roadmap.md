# Roadmap

## v0.1 — the development pipeline

v0.1 is validated against one concrete scenario: a five-agent development pipeline that takes a
goal over HTTP and ends with a pull request. It is specified in
[`routing.md`](routing.md#5-worked-example-a-development-pipeline).

```
HTTP ──▶ analyst ──▶ developer ──┬──▶ testing ──┐
                         ▲       └──▶ review  ──┴──▶ join(all) ──▶ announcer ──▶ PR
                         └──────── tests failed
```

> **This is a deliberately demanding target.** It is a significant expansion over "one agent can
> trigger another" and pulls fan-out, rendezvous joins, failure loops, workspace isolation and
> credential handling into the first release. Worth knowing before committing to it.

### Done criteria

**Configuration**

1. `layover.toml` parses: agents with prompt, model, runner, `access` and `entry`; routes with
   direction, fan-out and `join`.
2. Validation at config load rejects unknown agent names and unreachable entry points, and warns
   on fan-out to multiple read-write agents.

**Execution**

3. `layover run` starts the Tower — HTTP server and MCP server together.
4. `POST /flights` triggers the analyst in the shared workspace.
5. The developer fans out to testing and review, and they run **concurrently**.
6. The read-only review agent receives a git worktree snapshot, not the live tree.

**The join — the heart of v0.1**

7. `join = "all"` parks flights and spawns the announcer **exactly once**, with both inputs.
8. A testing failure loops back to the developer; the barrier resets; a full re-dispatch
   completes it with fresh results.
9. A barrier that can no longer be reached is abandoned, and the itinerary is marked **stalled**
   and shown as such — distinctly from failed.

**Safety and observability**

10. Hops decrement across the chain; a chain terminates at zero rather than running forever.
11. Hangars are written: transcript, run metadata, memory.
12. `GET /runs` and `GET /runs/:id/stream` return live SSE output.
13. A minimal UI renders the route map, a live run feed, and stalled itineraries.
14. `POST /ground-stop` halts a running factory and survives a Tower restart.

**The payoff**

15. The announcer opens a real pull request. Blocked on the credential decision below.

Criteria 7–9 are the ones that matter. Everything else is plumbing; the barrier is the difference
between a pipeline and a race condition.

### Out of scope for v0.1

- Resident agents
- `request_response` mode — **no longer needed**; rendezvous joins cover fan-in without holding
  processes open, which retired two risks rather than deferring them
- Fuel accounting
- Two concurrent read-write agents
- Anything multi-machine
- Pointing a factory at Layover's own source — permanently out of scope

---

## Open questions

Agents should ask rather than guess on any of these.

### Blocking for v0.1

1. **Which branch does the announcer open a PR from?** Branch-per-itinerary was declined, so
   nothing currently creates or names a branch. The announcer cannot open a pull request without
   one. Either the developer commits to a convention-named branch, or itineraries get branches
   after all.
2. **How do child CLIs receive credentials?** Inherited environment, or Tower-injected per run.
   Per-run injection allows per-agent credentials — an analyst has no business holding a token
   that can push.
3. **How many loop-backs before giving up?** Hops bounds it, but exhausting Hops mid-repair leaves
   a half-finished change in the workspace and no one to clean it up.

### Open

4. **What happens when a run *crashes*?** Distinct from a test failing: the process dies, exits
   non-zero, or times out. Retry, dead-letter, notify the sender, or stall the itinerary.
5. **Are prompts inline in TOML?** The pipeline's prompts are already multi-line and carry
   behavioural constraints. `prompt_file = "prompts/developer.md"` gives better diffs and lets
   agents edit them.
6. **Is the Logbook one flat file or namespaced?** One file serializes every write in the factory
   through a single lock.
7. **Observability** — structured logs, or OpenTelemetry traces where an Itinerary is a trace and
   each Run a span? The latter fits almost too neatly, and would visualise a stalled barrier.
8. **When is an Itinerary done?** The announcer opening a PR is success, but nothing marks it.
   Without a terminal state the UI cannot distinguish finished from idle.
9. **UI stack** — plain HTML with SSE, or a framework. Reversible; it only consumes the HTTP API.
10. **Is the aviation terminology confirmed?** Adopted provisionally throughout. Cheap to strip
    while there is no code.

### Resolved by the join design

- **Does an agent see who sent a flight?** *Yes — now mandatory.* The announcer receives two
  flights and must tell the test result from the review verdict. Sender identity became a
  requirement rather than a preference.

---

## Beyond v0.1

Rough ordering, not commitments.

- Fuel accounting, degrading to wall-clock or run count when a runner cannot report cost
- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- `join = "any"`, and quorum joins
- `include = [...]` for multi-file factory definitions
- Additional runners: Gemini CLI, Aider, arbitrary command templates
