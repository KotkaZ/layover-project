# Risks

Known hazards in the design, with the mitigation we intend. Severity is the risk *as currently
designed*, not as it would be with the mitigation applied.

---

### 1. Resident + reentrant conflict

**Severity: high.**

Fresh transient runs make re-entrancy safe — two concurrent runs of one agent share no state. But
a *pinned resident* agent has exactly one provider session, and two concurrent re-entrant runs
would resume and corrupt it.

*Mitigation:* resident agents serialize their runs; only transient agents are truly reentrant.
This makes residency a meaningfully different mode rather than an optimisation, which should be
documented as such.

### 2. Shared workspace contention

**Severity: high. Accepted for v0.1.**

Unbounded concurrent coding agents share one working directory and *will* clobber each other's
files. There is no locking, no worktree isolation and no merge strategy.

*Mitigation deferred.* The cheapest future option is a `writable_paths` allow-list per agent,
enforced by the Tower. Git-backed per-run worktrees are the thorough option and considerably more
work. Until then this is a known, deliberate limitation and must be stated plainly in user docs —
a factory of coding agents pointed at one repo is not yet safe.

### 3. Blocking chains hold processes open

**Severity: medium.**

A `request_response` chain of depth N keeps N CLI processes alive and idle, each holding memory
and possibly a provider session. Hops is the only bound on depth.

*Mitigation:* consider cutting `request_response` from v0.1 entirely. Async alone proves the
factory concept, and this risk plus risk 1 both stem from it.

### 4. Fuel accounting depends on the CLI reporting cost

**Severity: medium.**

Claude Code reports token usage in `stream-json`. Codex and Copilot CLI report less consistently.
If a runner reports nothing, Fuel silently stops metering and the budget rail becomes fiction.

*Mitigation:* Fuel must degrade to a secondary bound — wall-clock time or run count — whenever a
runner cannot report cost, and the Tower should log loudly when it does so. A safety rail that
fails silently is worse than no rail.

### 5. Agents rewriting their own static context

**Severity: medium.**

Agents given full write access to a workspace can edit that repository's `AGENTS.md` — the very
file shaping their behaviour. That is a drift loop with no human in it.

*Mitigation:* the Tower should treat instruction files as protected paths by default, overridable
in config. Note this applies to *users'* repositories; it does not apply here, because no factory
targets this repo.

### 6. Transcripts persisted but not exposed

**Severity: low.**

v0.1 can stream a live run but has no endpoint to read a finished one, despite storing full
transcripts and run metadata. Post-incident review would mean reading files by hand.

*Mitigation:* add `GET /runs/:id` and `GET /runs/:id/transcript` when the UI needs them.

### 7. Self-modification

**Severity: low — largely resolved.**

If a factory's workspace were Layover's own source, agents could edit the supervisor running them,
and a bad rebuild would brick the factory.

*Resolved by scope:* no factory ever targets this repository. The residual rule is that the Tower
always runs from an installed binary outside any workspace, never `cargo run` from inside one.

### 8. `layover.toml` as a single file

**Severity: low.**

Fine at ten agents, awkward at a hundred, and a merge-conflict magnet when several agents edit
the factory definition.

*Mitigation:* add `include = [...]` when it actually hurts. Not before.

### 9. Architecture drift between docs and code

**Severity: low, but grows.**

`docs/architecture.md` is normative and hand-maintained. Once code exists, nothing forces them to
agree, and an agent trusting a stale document will make confident wrong changes.

*Mitigation:* keep the decision log in `architecture.md` updated in the same commit as the change
it describes — this is why Conventional Commits and a `docs:` type matter.
