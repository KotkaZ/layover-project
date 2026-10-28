# Recovery, stopping and starting

Part of the [decision log](../decisions.md): restarting interrupted runs and provider errors, failed chains, draining, stopping and autostart.

**Why autostart generates rather than installs.** A lights-out factory that stops at every reboot
is not lights-out, so the Tower has to survive one. But registering a service writes to the
machine, and the artefact is exactly the kind of thing a person should read before it runs at
every logon. So `layover autostart` renders the platform's own file — Scheduled Task, launchd
agent, systemd user unit — and prints the one command that registers it. It also refuses to write
anything for a factory that does not load, because a service failing at every logon is worse than
no service. All three artefacts install into the logged-in user's session and never machine-wide:
the Tower spawns agents using *that user's* credentials, git identity and workspace, and a system
service would have none of them or would run as root with all of them.

**Why recovery and steering are the same mechanism.** They arrived as different requests — survive
a VM restart, and let a human redirect work in progress — and both looked like they needed a
process kept alive: resume the conversation, or pipe input into the running child. Either would
have made `resident` the normal case and brought back the reentrancy hazard that fresh runs
retired. Starting a *new* run seeded with a **handover** answers both, because what the second run
actually needs is not the first run's process but its context. Nothing is resumed, no session is
held open, and the locked "fresh runs" decision survives intact: a run is still a clean slate
process, it simply opens with more to read.

**Why recovery is bounded like every other rail.** A restarted run spends a hop, debits Fuel,
counts against the run cap and draws on the Reserve, and `max_recovery_attempts` bounds it on top
of all that. A crash loop that restarts itself forever is a fork bomb that looks like resilience.
A Ground Stop is never restarted through, because a factory that restarts past its own kill switch
is not one anybody can stop.

**Why Layover does not guess which agents are unsafe to restart.** Repeating work is harmless for
an agent that reads and reports, and dangerous for one that opened a pull request — a run
interrupted after pushing a branch but before recording it would, on restart, open a second. The
tempting check is "terminal and read-write", and it is a topological guess at a semantic property:
it fires on plenty of factories where repeating is fine, and a validator that cries wolf is one
people stop reading. So the mitigation sits in two better places. The handover text *tells the
agent* to check whether the earlier run already did the thing, which is the only party that can
actually look; and `recovery = "manual"` stops the restart outright for the steps where checking
is not good enough. The reference factory sets it on exactly one agent.

**Why recovery requires the previous process to be confirmed gone.** `Interruption` distinguishes
what the Tower *watched* from what it merely *inferred*. A timeout or a non-zero exit means it saw
the process end. A Tower restart or a lost pipe means only that it stopped being able to see one —
and on Windows a child routinely outlives the parent that spawned it. Recovering in that state
starts a second run beside a first that never stopped, which for a publisher means two pull
requests. So those interruptions carry `ChildState::Unknown` until something checks, and
`authorize_recovery` refuses them; the run record keeps the process id so there is something to
check. A recycled process id can make a dead run look alive, which fails towards refusing to
recover — the safe direction, because stalled work is visible and duplicated work is not.

**Why a restarting Tower stops a run it finds still alive.** Such a run is cut off: its MCP endpoint
and token died with the Tower that minted them, so nothing it sends, reports or books can arrive,
and left alone it keeps spending on work nobody will receive. The alternatives were weighed: letting
it run to completion in a held slot, or doing that only for agents whose `recovery` is `manual`.
Both leave the chain dead-ended — its output has nowhere to go — and pay for work that is thrown
away. Stopping it is also what makes it *confirmed* gone, which recovery requires; the restart is
then decided by the agent's `recovery` policy and `max_recovery_attempts` as for any interruption,
and is told to check before repeating a side effect. Decided with the maintainer on 30 September
2026.

**Why each Tower holds a lock rather than recording its process identifier.** A Tower settling runs
must leave alone those another living Tower is watching — `layover run` beside a `serve`. Comparing
process identifiers invites the reuse problem recovery already had to solve for runs. A lock on a
file of its own is released by the operating system however the Tower ends, so "is anybody holding
it" is exactly "is that Tower alive", and a record written by a release that kept no owner is
treated as a Tower that has gone.

**Why a restart is charged what the run it replaces spent.** A chain's ledger lives in the Tower's
memory and starts afresh after a restart. Charging the interrupted run's measured spend — and
counting it as a run — before the restart is admitted keeps recovery a rail rather than a way for a
chain to buy itself a new budget by being interrupted.

**Why a released join is restarted past its barrier.** The barrier state went with the Tower that
held it, and decisions above discard it on restart. Delivered to a fresh barrier, the restarted join
would wait for upstreams whose runs finished before the restart and be given up as unreachable. So
the live record keeps the one flight carrying every arrival, marked as already released, and a
restart delivers it straight to its agent.

**Why a run the model service ended is restarted, and no other failed run is.** A production factory
lost three DevForge chains in a day the same way: a run that had done its work — one had passed a
35-minute verification matrix — made one more model call, the service answered
`400 internal server error`, Copilot CLI does not retry a 400, and the process exited 1. Recorded as
`failed`, it was indistinguishable from a run whose tests failed, and nothing restarted it, because
recovery covered only runs alive when the Tower went away. But the two failures want opposite
things. A run that failed its *own* work — a test that failed, a command its runner denied — would
fail the same way again, so restarting it spends money to arrive in the same place, and in a loop
between two agents it would spend it until a rail cut it off. A run the *service* dropped did not
fail its work; its work was interrupted, and that is exactly what `recovery` exists to decide about.
So a provider error is treated as an interruption — the agent's `recovery` policy,
`max_recovery_attempts` counted per piece of work across every restart of it, a Ground Stop
forbidding it, an ordinary run on every rail — and every other non-zero exit is left as the failure
it is. `first-release.md` records that "a crash reuses the recovery policy"; restarting every crash
was never built, and this narrows it on purpose to the one kind of crash that is not the run's.

**Why a provider error is read from the CLI's events, and from the end.** A pattern over the output
would match an agent *quoting* an error — a tool result holding a log that says `session.error`, a
message explaining a 400 — and restart a run because it read about a failure. Only the top-level
fields of each JSON event are matched, so a quote, which is a string inside another event, never
does. And the failure has to be the last thing that happened: Copilot CLI recovers from many failed
calls itself, reports a sub-agent's exhausted retries with the session's own error, and in one real
run went on — woken by a background task — to finish its work after a 400 and then exit 1 for it.
So a failure counts only with no tool started and no call succeeding after it. One failure is not an
event: Copilot CLI checks the model it was given before it makes a call, and a model the service is
not offering at that moment ends the run with a line of plain text, `Error: Model "gpt-6.1-sol" from
--model flag is not available.` A short outage on 26 October 2026 did that to the run carrying a
chain's only pending work, and because the line was not an event it read as the run's own failure:
nothing restarted it and the chain sat silent for seventeen hours. That line counts too, under the
same rule, and it is still safe from a quote: it must be a whole line of the transcript, in a known
wording kept in one place, and an agent quoting it does so inside an event, whose line starts with
`{` and is never compared. A model name the CLI does not have at all fails that way on every
attempt; the attempts and back-off bound it, the record keeps the CLI's words, and `layover doctor`
names an agent whose last runs were all ended so. Claude Code and Codex have no reader yet, because a guessed one would restart runs that failed for their own reasons; a
runner can declare the events instead, as fields an event must have rather than a pattern.

**Why the restart waits.** A provider fault often comes in a cluster; restarting into the same
minute would meet it again and spend an attempt on nothing. A minute before the first restart, five
before the second, fifteen after that: long enough to step out of a cluster, short enough that a
night's work is not lost to it. The wait is kept on the queued flight rather than in a timer, so a
Tower restarted while it waits still waits — and one that comes back after it is due starts it at
once. A restart after a Tower restart waits for nothing, because what interrupted it is over by the
time a Tower comes back.

**Why a `manual` agent's run that started no tool is restarted after all.** On 28 October 2026 the
model service answered 401 for a few minutes. Two runs of an agent marked `manual` failed in it, and
one was the hourly tick collecting a layover its agent had booked to follow a pull request: its
transcript was start-up events and the error — no tool started, no call succeeded. `manual` left it,
the layover was spent, nothing booked it again, and the pull request stopped being followed without
anybody being told. `manual` exists so that work which must not happen twice is not repeated, and
every side effect an agent has — a command, a file, `layover_send`, `layover_book`, any MCP call —
is a tool it starts, which Copilot CLI reports as a `tool.execution_start` event and, when it ends,
a `tool.execution_complete`. A run with neither did nothing, so restarting it cannot repeat
anything, and leaving it only loses the work. So it is restarted, counted on the same attempts,
after the same back-off, on every rail, and never through a Ground Stop; its record and its chain
say why. It needs a reader that sees tools: the count is taken from the same top-level events as the
failure, never from text, and either event counts as proof a tool started, because this reads an
absence — the weaker evidence — and one event renamed in a later Copilot CLI must not make a run
that acted look untouched. A declared reader, which names failures and signs of carrying on rather
than tools, cannot show that none started, so its runner's `manual` agents keep the old rule. Only a
provider error: a Tower that went away read none of its run's events. `never` still means never.

**Why a `manual` agent's provider failure that is not restarted files a help request.** The run that
was left — it had started a tool, its attempts were spent, a Ground Stop was engaged — was a failed
run in history and a failed chain, and the help queue, the one place built to reach a person, said
nothing; it was more than an hour before anybody noticed. So the Tower files a help request for the
agent: not fatal, since nothing waits on an answer, blocker `environment`, naming the run, its chain,
the provider's request id, how many tools it had started and, when it was collecting a layover, that
the layover is spent. It records the chain's workflow and flags, so a reply starts the agent again as
that chain would have. Only for `manual`: an `automatic` agent's work restarts by itself, and one
whose attempts run out is in `layover doctor`.

**Why restarting provider errors is on by default.** `recovery` already says, per agent, whether
doing its work twice is safe, and it defaults to `automatic`; a separate switch for provider errors
would be a second knob for the same question. A factory that marked its irreversible agents `manual`
— as the reference factory and the maintainer's do — is already protected, and one that wants the
old behaviour has three ways back: `max_recovery_attempts = 0`, `recovery = "manual"` per agent, or
`provider_errors = { read = "none" }` per runner. The other default — off, until a factory opts in —
would leave every existing factory losing chains exactly as before, which is the problem.

**Why a chain that ended on a failed run is a state of its own.** Restarting provider errors removes
the commonest way a chain dies silently, not every way. A run that fails, times out or is
interrupted and not restarted never sends the flight that would carry its work on, so nothing is
queued, nothing is stalled at a barrier, nothing was refused — and the chain read as `finished`. So
`failed` joins `stalled` and `awaiting_human` as a state a list of runs cannot show: the chain's
last run, by when it finished, ended badly, and nothing came after it. "Last by finish" so that a
fan-out branch that failed while another went on to the end is not the chain's end. A chain somebody
continued is `finished`, because a person dealt with it.

**Why a dismissed chain keeps its state, and why a dismissal names a run.** Continuing was the only
way to clear a `failed` or `stalled` chain, so one fixed by hand, or no longer wanted, stayed in
*needs attention* and the red count until it aged out of the window — and a count that cannot be
cleared stops being read, which is the failure the state exists to prevent. A dismissal is the
chain's counterpart of resolving a help request. It does not make the chain `finished`: that would
rewrite what happened, and the state is evidence somebody may look for later. So it sits beside the
state and only the counts that ask for a person leave it out — `stalled` and `failed` still count
it, with `dismissed` beside them. It names the run the chain had ended on, not the chain, because
that run's failure is what a person looked at and judged; a chain that is restarted and fails again
has a failure nobody has seen, and a dismissal of the chain would hide it. Only `stalled` and
`failed` can be dismissed: a chain waiting for an answer already has its own ways out, and a halted
one is held by something a person releases.

**Why a run 1.3.0 lost ends when its transcript was last written.** Its record says only that it
was started. The time the next Tower noticed it is the one thing certainly *not* when it ended — a
run that died twenty-nine minutes in read as taking almost six hours. The transcript is appended to
for as long as the CLI runs, so its last write is the last evidence of life. Its workflow comes from
whatever else records the chain — another run in history, queued work, a layover, a help request —
and stays empty when nothing does, because a guess would file it under a workflow it never belonged
to.

**Why the first Ctrl+C drains, and only the second cuts off.** `serve` used to end its process at
the first Ctrl+C. Every live run was cut off, recorded `interrupted` by the next Tower and restarted
from the top, so stopping the Tower to change its configuration cost the work and the spend of
whatever was running; and an arrival parked at a rendezvous, which exists only in the Tower's
memory, was simply lost. A Tower that cannot be stopped without damage cannot be updated without
damage either, which is what made this the first step of updating Layover. So the first Ctrl+C now
drains: nothing new starts, what is running finishes with its MCP endpoint still up, and the process
exits once the factory is quiet. Queued work is left in the queue rather than run, because a drain
that runs the queue dry may never end in a factory that keeps itself busy. A second Ctrl+C cuts off,
because a person who presses it twice means it; and a drain has no deadline of its own for the same
reason — the person watching it decides. Cutting off kills each live run's process tree rather than
leaving it: a run whose Tower has gone can no longer send a flight or be recorded, but it can still
push a branch, so it must not outlive the Tower that could hear from it. It is not written down — a
run killed halfway has no outcome of its own — and its live record is left for the next Tower to
settle as `interrupted`, the same path a crash takes, so recovery has one way in rather than two.
Every way of stopping exits 0: the Tower stopped because it was asked to, and the systemd unit and
launchd agent `autostart` writes restart a Tower that exits non-zero. The one exception to "nothing new starts"
is a flight that completes a rendezvous already holding an arrival, in the same chain: refusing it
would lose the arrival, which is the damage draining exists to avoid, and such a flight cannot loop,
because the barrier releases once. A schedule's tick that comes due while the Tower drains is held,
so if the stop is called off it fires once rather than being skipped.

**Why stopping is asked for with a file.** `layover stop` writes `.layover/stop-request.json`
rather than calling the HTTP API: the API needs the token, which a script stopping the factory
should not have to scrape from a log, and a file works with the dashboard's port unknown, the network
down, or the Tower started by the operating system without a terminal anyone can press Ctrl+C in.
It is the Ground Stop's mechanism for the same reasons. A Ctrl+C writes the same file, so a dashboard
in another process can say the Tower is stopping however it was asked. A request found when a Tower
starts was written for one that has gone, and honouring it would stop a Tower nobody asked to stop,
so it is removed and reported. An unreadable request is a request to drain rather than to cut off,
because when it is unclear what was asked, the gentler reading loses nothing.
