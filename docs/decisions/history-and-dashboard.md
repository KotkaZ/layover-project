# History and the dashboard

Part of the [decision log](../decisions.md): history on disk, reporting windows, the route map, watching a run, and doctor.

**Why history is a directory of text files rather than a database.** History is append-only,
written once and read in whole windows — the one shape a query planner is least needed for. The
cost would have been real: `rusqlite` bundles a C library, which means a C cross-compiler for
each of the five targets Layover releases to, trading the musl and aarch64 builds for indexing
that a few megabytes of JSON does not need. Segmenting by day buys three further properties a
single growing file would not: retention becomes deleting files rather than rewriting one, a
window opens only the days inside it, and a partial write — which is what a power cut leaves —
costs one line instead of the file.

**Why the segments are UTC days while the reports are local.** Reporting windows are local
because "this month" is a local question, but a local-day *filename* shifts when the machine
changes zone or the clocks go back: two days would want the same name, or one day would be split
across two files. UTC has no such day. Reading a local window therefore opens one extra segment at
each end and filters by instant — cheap, and it cannot be wrong.

**Why a window is either rolling or calendar, in the type.** "The last 7 days" and "this month" are
asked in the same breath and are not the same kind of question: the first is 168 hours everywhere on
earth, the second begins at a midnight that depends on where the Tower is standing. Collapsing them
is how a factory spends one day's money twice — an earlier agent factory gated on a UTC boundary
while reporting in local time, and for the hours between the two midnights the gate and the display
described different days. So `Window` distinguishes them and every resolved `Span` carries the zone
it was reckoned in, or `None` when there was nothing to reckon. The absence is as informative as the
name.

**Why the dashboard draws its own graph instead of using Mermaid.** Mermaid stays for
`layover graph`, where the output is pasted into a README and portability is the whole point. It
was rejected for the dashboard on weight: the runtime is 2.5 MB of JavaScript that would have to
be vendored into the repository and embedded in the binary to keep the page working offline. A
layered layout for twenty nodes is a few hundred lines that can be unit-tested, where asserting
on a JavaScript library's rendering could not be. Layers come from breadth-first distance rather
than longest path, because route maps are routinely cyclic — the review loop is the point of the
reference factory — and longest-path layering does not terminate on a cycle. Reusing the distance
the validator already computes also means the diagram's columns and the hop arithmetic can never
disagree.

**Why the dashboard refuses control operations instead of hiding them.** Sending a flight,
streaming a run and engaging a Ground Stop all needed a supervisor that did not exist yet, and
they answered `501` rather than returning something plausible. A control that silently does nothing
is worse than a control that is not there: it is trusted once, and then relied upon at the moment
it matters. Each was built when the Tower could do it; streaming was the last.

**Why a run is watched by reading its transcript, not by asking the Tower.** The Tower already
streams every run's output to a file in its Hangar, so following that file is enough — and it is
the only way that works from a `--watch-only` dashboard, or one beside a `serve` in another
terminal, which share nothing with the Tower but its directory. It also makes a finished run and a
live one the same request: replaying how an agent reached its conclusion is reading the file from
the start and not waiting at the end. The same goes for knowing what is running: history holds a
run only once it is over, so the dashboard reads the Tower's live records, which is why they moved
from `layover-tower` to `layover-store`.

**Why the transcript is rendered on the server.** A forty-minute Copilot review wrote 44 MB, of
which 29 MB were token-by-token deltas of text that then arrives again whole, and single events ran
to 280 KB. Sent raw, the browser would parse all of it to show a fraction, and the rendering — which
event means what, in which CLI's dialect — would be untested JavaScript. In Rust it is a pure
function over lines with tests against the real event shapes, it is redacted once in the same place
as every other excerpt Layover shows, and the page receives a few hundred kilobytes. Deltas are not
thrown away: while a block is still arriving it is sent as a *partial*, which the finished event
replaces, so a live session still shows the model typing.

**Why watching is read-only.** Typing into a running agent is steering, and steering is settled as
a new run carrying a handover, never input to a live process. A terminal that looked interactive and
was not would invite exactly the wrong expectation, so it says *read-only* on every pane.

**Why the dashboard asks the Tower when a schedule next fires.** A cron expression could be
evaluated anywhere, but an `every` schedule counts from when the Tower started, and a tick held by
a Ground Stop fires when it is released — neither is in `layover.toml`. A page that worked the
times out for itself would show a timetable that looks exact and is wrong by however long ago the
Tower started. So `serve` hands its dashboard the clock it ticks, and the same "still working"
answer the clock consults before it skips, and a `--watch-only` dashboard says it has no clock
rather than guessing. Skipped ticks are written to the journal as well as printed, because a
schedule that skips every tick leaves a history that looks healthy — few runs, none failed — and
the console line was the only evidence.

**Why history files a run by the day it finished.** The run was written by hand into the segment of
the day it *started*, while the history reader looks for a run by its finish. An hour-long run that
crossed midnight landed in a segment its own window never opened, and parallel runs appending by
hand could interleave. Every run now goes through `History::append`, which files by finish and takes
the segment's lock.

**Why `doctor` measures free slots rather than waiting time.** Work waiting in a busy factory is the
design. What the one-at-a-time Tower produced was waiting *with slots free*, so that is what is
measured: from history, how long each run spent queued while fewer than `max_concurrent_runs` runs —
and fewer than its agent's own cap — were alive. Over `timeout_sec` of that is a warning.

**Why the dashboard still draws one diagram per workflow once routes are scoped.** Scopes invite a
combined map with each edge styled by workflow, and the page does not draw one. Separate diagrams
were chosen because a combined one reads as a single confused process, and scoping makes that
worse, not better: the reader now has to filter edges by colour to see what one workflow can do,
which is the separation the per-workflow diagram does for them. So each workflow is drawn over its
own routes and nothing else. The whole-factory picture still exists for the question it answers —
"what may anything reach, anywhere" — as `layover graph` and `GET /graph` without a pipeline, and
there a scoped edge is labelled with its pipelines, so it is never mistaken for a global one.

**Why a route each way is drawn as one line.** Most routes in a real factory come in reciprocal
pairs, and the layered layout drew every reverse direction as a return path: a loop under the
whole diagram in a lane of its own. A twenty-route workflow became eleven loops stacked four
hundred pixels deep, crossing everything on the way, and the map stopped answering the question it
exists for. Merging a pair into one two-headed line loses nothing — the line says exactly "either
may send to the other" — so long as only *plain* pairs are merged: a join, a spawn or a differing
scope makes one direction mean something the other does not, and each keeps its own arrow. A route
between two agents in the same column gets a short connector or arc beside the column for the same
reason; only a route that genuinely runs backwards still goes round the bottom. Interaction —
hovering an agent to light its routes — was added too, but not as the fix: it helps someone who
already knows which agent to ask about, and a map has to be readable before anyone touches it.

**Why one chain is drawn on its workflow's map, and why a run records who sent it.** Once runs
overlapped, a workflow triggered three times was still one drawing, coloured while *any* of its
chains ran an agent — so "the coder is running" stayed true and stopped saying anything. A view per
chain was the fix, and it reuses the workflow's own layout rather than drawing the chain as a
timeline or a fresh graph: the same boxes in the same places is what lets a person compare where
this run went with where the workflow can go. Lighting the routes a chain took needed something
history did not hold. A run record said which agent ran, not whose flight woke it, and inferring a
route from "the coder ran after the analyst" draws edges that were never taken the moment two
agents can both send to a third. So the Tower now records `sent_by` — every arrival, for a released
join, because naming the first would draw one of the two routes that led there. Three answers are
kept apart on purpose: agents, nobody in the mesh (`[]`, the way in), and *not known* (`null`), for
records written before this and for a restarted join whose arrivals left with the Tower that held
them. Not known lights nothing; a route drawn from a guess looks exactly like one that was taken.
The chain's whole picture is one endpoint rather than four because the page refreshes it every few
seconds, and four answers read at four moments can disagree about whether a run has ended. Parked
barrier flights are not drawn: they live only in the Tower's memory, and a dashboard that guessed
at them would misreport the one wait somebody is most likely to be watching.
