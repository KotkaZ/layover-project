# Running agents in parallel

Part of the [decision log](../decisions.md): slots, the coordinator and its clock, the order work starts in, shared files and ending a run.

**Why Slots queue where every other rail refuses.** Hops bounds depth, Fuel and the Reserve bound
money, the run cap bounds a chain's total — and none of them bounds how many agent CLIs are alive
at one instant, which is the number that takes a machine down. The gap surfaced by writing a real
pipeline: a scanner dispatching one reviewer per pull request assigned to you has a width nobody
knows until it looks, and fifty assigned pull requests meant fifty simultaneous processes with
every rail satisfied. Slots queue rather than refusing because every other rail protects a budget,
and money spent is gone, whereas a machine that is busy now will not be busy in a minute. Refusing
would turn "review twelve pull requests" into "review four and silently drop eight", which is the
worst available reading of a concurrency limit.

**Why one coordinator decides and many threads wait.** Until 1.4.0 the Tower ran one agent at a
time while `max_concurrent_runs` was parsed and read nowhere: an Eagle Eye sweep of ten hour-long
reviews took ten hours, and a 14:00 schedule fired at 14:23 because the clock waited for the run in
front of it. The fix keeps everything that decides *whether* work may start on one thread —
unqueueing, barriers, admission against Hops, Fuel and the run cap — and moves only the waiting,
which is where all the time goes, onto a thread per run. So the guarantees the sequential loop gave
hold without a lock around them: a flight leaves the queue before it runs and is never taken twice,
a barrier sees its arrivals one at a time, and a chain's rails are charged in the order its flights
were admitted. A chain's ledger is held only while it is charged — admission before the spawn, the
Fuel debit after the exit — so runs of one fan-out alive at once each see what the last left.

**Why the clock ticks on the dispatcher's thread.** A schedule skips a tick while its last wave is
queued *or* running, and a flight moves from one to the other as it starts. A clock on a thread of
its own could look at both between those moments, see neither, and start a second copy of work
already under way. Ticked from the dispatcher between one step and the next, nothing moves while it
looks. The dispatcher looks for new work every quarter second while runs are alive, so schedules
still fire on time.

**Why the next flight is the oldest one that can start.** First in, first out, among flights whose
agent is below its own `max_concurrent`. A flight for an agent at its cap waits where it is, and
flights behind it for other agents go ahead: waiting for the one agent is what the cap asks for,
and holding the whole factory behind a single Teams sender is not.

**Why the slot count comes from the runs alive rather than from `Slots`.** `Slots` keeps a waiting
line of run identifiers, but a queued flight has no run yet, and the line work actually waits in is
the journal's queue — durable, and read back after a restart. A second line in memory would be a
second source of truth that dies with the process. So the dispatcher counts the runs it holds alive
against the limit and takes the next eligible flight off the journal's queue. The `Slots` type,
written before the dispatcher and never used by it, has since been removed.

**Why shared files are locked per path, in one registry.** Runs of one agent now overlap, and they
share its Hangar's `memory.md`, the factory's learnings and logbook, and the journal's help, reports
and queue. Each read-modify-write takes a lock keyed by the file's path from one process-wide
registry, so two handles on the same journal share it, and each appended line is one write of the
whole line. Across processes nothing is shared, which is why two Towers over one factory remain
unsupported — and, since 1.15, refused: see risk 23.

**Why a run is also ended without asking `kill`.** On Unix the Tower ended a run by running
`kill -KILL -<pid>`, and on Ubuntu 24.04 that never worked: procps 4.0.4 reads a negative number as
options, signals the group named by its first digit, and reports success when that fails. The tests
said "timed out" throughout, because they checked the outcome and not how long it took — thirty
seconds for a 300-millisecond timeout. The group is now addressed after `--`, which every `kill`
checked reads as a pid, and the Tower then signals the process itself through the standard
library, which depends on no external command at all. Its own children are another matter: see risk
24.
