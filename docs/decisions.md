# Decisions

Why Layover is built the way it is, and what is still undecided.

This file exists because there is no ADR trail: an agent opening this repository starts fresh and
cannot recover intent from a diff. The reasoning has to be written down somewhere, and one place
is better than two — a decision recorded in two documents is a decision that will be revised in
one of them.

> **Settled questions are not listed separately.** Each one is a decision, and the decision log
> below carries it *with its reasoning*, which is the more useful record. A second index of the
> same answers only offered a second thing to forget to update.

System design lives in [`architecture.md`](architecture.md); route map semantics in
[`routing.md`](routing.md); known hazards in [`risks.md`](risks.md).

---

## Decision log

Each entry is a decision that was not self-evident, with its reasoning. They are grouped by
what they are about, and within a file kept in the order they were made:

| Topic | What it covers |
|---|---|
| [The shape of the system](decisions/foundations.md) | Supervising CLIs, MCP, fresh runs, the Logbook, the generated HTTP server, licence, toolchain and documentation |
| [Routes, joins and reach](decisions/routes.md) | Joins and barriers, Hops at load time, scoped routes and what validation checks |
| [Workflows, triggers and spawning](decisions/workflows.md) | Pipelines, schedules, manual triggers, spawning, layovers, follow-ups and per-workflow rails |
| [Prompts, flags and what a run is told](decisions/prompts.md) | Conditional prompts, the sender, flags and names that travel with a chain, and reports |
| [Runners, agents and configuration](decisions/runners.md) | MCP servers per agent, credentials, the prompt on stdin, presets, models and effort, and the CLI's home |
| [Cost, Fuel and the Reserve](decisions/cost.md) | Where a cost figure comes from, Copilot credits, rate cards, the Reserve and the run cap |
| [History and the dashboard](decisions/history-and-dashboard.md) | History on disk, reporting windows, the route map, watching a run, and doctor |
| [Recovery, stopping and starting](decisions/recovery.md) | Restarting interrupted runs and provider errors, failed chains, draining, stopping and autostart |
| [Running agents in parallel](decisions/concurrency.md) | Slots, the coordinator and its clock, the order work starts in, shared files and ending a run |
| [Learnings and help requests](decisions/learning.md) | Learnings that apply at once and lapse, help requests and the replies to them |
| [Updating Layover](decisions/updates.md) | Looking for, verifying, staging and installing releases, and rolling them back |

A new decision goes at the end of the file it belongs to. One that belongs to none starts a new
file, listed here.

---

## Still open

Fifty questions were answered on 18 September 2026; the reasoning is in
[`first-release.md`](first-release.md). What follows is what remains genuinely undecided.

Agents should ask rather than guess on any of these.

1. **Idle detection.** Whether "no output for N minutes" should be a rail alongside wall-clock
   `timeout_sec`. It needs per-runner knowledge of what counts as output, and for unattended
   spending the distinction between thinking and wedged is worth real money. Deferred, not
   dismissed.
2. **Two-phase Fuel reservation.** The floor is an approximation; reserving before a run and
   reconciling after is the real answer. Recorded as risk 15.
3. **Dispatch-aware barriers.** Wanted the moment a factory has several rarely-needed expensive
   helpers. Held back by a race that needs a dispatch window.
4. **A code of conduct.** Absent deliberately; the argument for adding it strengthens the moment a
   first outside contributor appears.
5. **Continuity.** A bus factor of one, stated plainly rather than solved.
6. **How a Codex run is told about MCP servers.** `codex exec -c` accepts only a dotted
   `key=value` override — `-c mcp_servers.layover.url="…"` — and has no flag that reads a file, so
   the documented `mcp = { flag = "-c", format = "codex_toml" }` wiring passes a path Codex refuses
   and every Codex run with MCP fails before it starts. Layover writes the right *content*
   (`[mcp_servers.<name>]` tables, with `env_vars` and `bearer_token_env_var` so no secret is in
   the file) but nothing can hand it over. The options: expand the file into one `-c` pair per key
   (no secret needs to reach the command line, since the token and credentials are all named by
   variable); point `CODEX_HOME` at a per-run directory, which would also hide the operator's own
   Codex login; or drop Codex MCP wiring until Codex reads a file. *Recommendation:* the `-c`
   expansion, as a new `format = "codex_overrides"` so the runner still says what it wants, tested
   against a real `codex exec` before it ships.
7. **Credentials for an HTTP MCP server.** `env_from` on a `url` server puts the variable in the
   agent CLI's environment, which a remote server cannot read, and there is no way to say "send
   this variable as the `Authorization` header". The reference factory's `ado` server declares
   `ADO_PAT` and would reach Azure DevOps unauthenticated. Both Claude Code and Copilot CLI expand
   `${NAME}` in `headers`, so a `headers_from = { Authorization = "Bearer ${ADO_PAT}" }` — names
   only, validated like `env` — would keep the secret out of every file. *Recommendation:* add it,
   and until then have `validate` warn that `env_from` on a `url` server does nothing.
8. **What a worktree is, before any is built.** `access = "read-only"` and `workspace =
   "per-itinerary"` are declared everywhere and implemented nowhere: every agent runs in its
   `work_dir`. The decisions above settle *that* they get worktrees; these settle *how*, and each
   changes what the reference factory does:
   - **Which tree a read-only snapshot starts from.** "The current commit" does not contain a
     developer's *uncommitted* change, which is exactly what the tester and reviewer are asked to
     judge. Either the developer must commit before handing off (a prompt rule, fragile), or the
     snapshot is taken from the working tree including uncommitted changes (a copy, not a
     worktree), or read-only agents in a chain share that chain's writer's tree read-only.
     *Recommendation:* in a `per-itinerary` pipeline, read-only agents run in the itinerary's own
     worktree — the tree they are judging — and a separate snapshot is only taken for a
     `read-only` agent in a `shared` pipeline, from `HEAD` plus a note in its payload that
     uncommitted work is not visible.
   - **One per itinerary, or one per run.** Per run is safest and costs a checkout per test run;
     per itinerary is cheaper and lets a loop's reviewer see the tester's build.
     *Recommendation:* per itinerary for `per-itinerary` pipelines, per run for a read-only agent
     in a `shared` one.
   - **Cleanup.** When the itinerary goes quiet — no run live, no barrier parked, nothing queued —
     or only after a retention period, so a stalled chain's tree can be inspected. And what happens
     to a branch the worktree created. *Recommendation:* keep it until the itinerary is quiet and a
     day has passed, prune with Hangars, never delete a branch with unpushed commits.
   - **A `work_dir` that is not a git repository.** Refuse at load for a factory that declares
     either setting, fall back to the shared tree with a warning, or copy. *Recommendation:*
     `validate` errors when `work_dir` is not a repository and isolation is declared, because a
     silent fallback is the state the project is in today.
   - **A resumed layover**, which opens a new itinerary about an old one's pull request: new
     worktree from the pull request's branch, or the old itinerary's kept tree.
9. **Two global routes that disagree about one edge.** Scoped routes that a chain could use
    together must agree about `mode` and `join` for any pair they share, and `validate` refuses
    them when they do not. Two *global* routes naming the same pair, one spawning and one not, were
    never checked: the edge silently spawns. It was left alone so that no existing factory's
    findings changed. *Recommendation:* report it as the error it is at the next major version,
    with the same message the scoped case uses.
10. **Checking a resumed chain against what it can actually reach.** Validation measures a
    resuming pipeline's reach, hop depth and flags from its `entry`, as it always has — but resumed
    work goes back to whichever agent booked the layover, and the chain is narrowed by its booking
    pipeline. So a resumed agent whose prompt tests a flag the resuming pipeline does not declare
    still fails at composition, not at load. *Recommendation:* for each resuming pipeline, check
    the flags of every agent any chain can wake (the same starting set the dead-route check uses);
    it can only add findings, which is why it waits for a major version.

## Not yet built, and not yet decided

Rough ordering, not commitments.

- Safe concurrent read-write agents: path allow-lists or per-run worktrees
- Resident agents with serialized runs
- Quorum joins, and joins that wait only for the upstreams actually dispatched (`all` and `any`
  exist)
- Non-boolean pipeline parameters, if booleans turn out not to be enough
- `include = [...]` for multi-file factory definitions
- Two-phase Fuel reservation, replacing the floor
- Idle detection alongside wall-clock timeouts
- Authenticode and Apple notarization, once the warning costs more than the keys
