# Cost, Fuel and the Reserve

Part of the [decision log](../decisions.md): where a cost figure comes from, Copilot credits, rate cards, the Reserve and the run cap.

**Why there is a Reserve as well as Fuel.** Fuel bounds one itinerary, and that turned out not to
bound the factory. A scheduled pipeline mints a *fresh* itinerary — and a fresh Fuel budget — on
every tick, so an hourly pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day with every
individual chain sitting perfectly inside its rail. The rail was real and the arithmetic still
ran away. The Reserve is the missing axis: a ceiling on total spend that no per-chain budget can
reset.

**Why the Reserve rolls instead of resetting daily.** A daily cap sounds simpler and is worse twice
over. Midnight doubles it — spend the cap at 23:59 and the bucket resets a minute later, so "$50 a
day" permits $100 in two minutes. And a day needs a timezone: an earlier agent factory's daily gate
bucketed by UTC while its ledger bucketed by local time, so between local midnight and the UTC
offset the gate read the wrong day's total and let spending through. "At most $50 in any rolling 24
hours" has no midnight, no timezone and no daylight-saving edge, and is strictly stricter.

**Why a cost figure carries where it came from.** A number can be reported by the runner, derived
by Layover from token counts and a rate card, or absent entirely, and those are not
interchangeable. Collapsing them is how a budget quietly becomes fiction: the same earlier factory
priced runs from a hand-maintained table and ran **2.7× over actual** — billing one model at `$75`
per million output tokens where the provider charged `$25` — with nothing in the totals saying
"this is a guess". So `CostSource` is a field, not a comment; a total reports the *weakest* source
that fed it, and a summary that is 90% measured still reports as an estimate. For the same reason
Layover ships no rate card: prices change per provider and per context tier, and a stale table
baked into a release is precisely how the drift happens.

**Why Copilot runs are priced from their credits.** Copilot CLI prints no dollars and no token
totals, so every Copilot run was `unreported`: Fuel debited nothing, the Reserve counted nothing,
and a Copilot factory was bounded only by its run cap and its timeout. It does print its usage —
`session.usage_checkpoint` events with a running total of AI units, `totalNanoAiu` — and GitHub
publishes the price of that unit: one AI credit is one US cent. So a Copilot run is priced at the
last checkpoint's credits times `[copilot] usd_per_credit`, and recorded as its own source,
`copilot_credits`. Three alternatives were weighed and rejected:

- **Premium requests**, the unit this entry's open question once recommended. They are a flat
  model multiplier per prompt — Opus 5.5 reported 15 for a 47-minute run and for a 6-minute one
  alike — so a budget in them cannot tell a long run from a short one, which is the one thing a
  budget is for. Never priced, not even as a fallback when no checkpoint was seen.
- **A separate budget counted in credits**, beside dollars. Deterministic and needing no price,
  but a factory mixing runners would have two budgets neither of which saw the other's spend.
- **A rate card entry**, labelled an estimate. The count is not an estimate — Copilot measured it
  — and labelling it one would make every Copilot total read as a guess.

The default rate is a named constant rather than something the operator must supply, which looks
like it breaks "Layover ships no rate card" and does not: that decision is about per-model token
prices, which drift per provider and per tier; this is the published size of the unit Copilot
counts in. A factory billed at another rate overrides it, and `validate` refuses zero, negative and
non-finite rates, because zero would record every Copilot run as free *and* measured. That one AI
unit is one AI credit is an assumption, from the CLI's own text output labelling them "AI Credits";
it is why the source is named apart from `reported` — if the assumption is ever wrong, the affected
runs can be found and repriced.

`copilot_credits` is measured: it debits Fuel and draws on the Reserve like `reported`, counts
towards `measured_share`, and is not "reported nothing" to `layover doctor`. A total that includes
it has confidence `copilot_credits`, weaker than `reported` and stronger than `rate_card`, and the
dashboard names it — "3 of 5 runs priced from Copilot credits" — so the arithmetic stays visible.
Everything else is unchanged: a last checkpoint that cannot be read, reads as negative or is not an
integer makes the run `unreported` rather than priced from an earlier total; zero credits beside
premium requests is silence; and a run killed before its first checkpoint is `unreported`, and
every total built on it a lower bound.

**Why a rate card prices only silence, and moves no rail.** The card was parsed and validated and
then never applied, so a Codex factory that had written one still showed no spend. Applying it
raised two questions. Which runs: only those that printed tokens and no dollar figure at all — a
printed figure that was disbelieved stays `unreported`, for the reason first-release.md gives for
not clamping one to a card. And what an estimate may move: nothing. Fuel and the Reserve bind on
measured figures only, so typing a price into a configuration file cannot make a budget trip, or
fail to — the run cap and the timeout remain what bounds a runner that does not price itself. The
model is the one the command line selects, as the route map reads it, because pricing a model the
runner never passed would be pricing a run that did not happen.

**Why `validate` warns about a runner's output mode, and about Codex only beside a rate card.** A
runner started without the output its CLI prints a cost in is the commonest way a factory's spend
becomes unknowable, and nothing fails when it happens — so it is said before the first run rather
than discovered from a dashboard reading zero. Only what a change to the command would fix is
warned about. Codex prints no price however it is run; a warning nobody could ever clear is one
people learn to skip, and would make `validate --strict` fail every Codex factory for good. So
Codex is flagged only where a rate card is waiting for the token counts `--json` would give it, and
`layover doctor`, which sees the runs that actually reported nothing, says the rest. The CLI is
recognised by name anywhere in the command, because factories wrap them (`cmd /c copilot`,
`npx @github/copilot`); a command naming none is not judged, because a stand-in or another CLI may
print whatever it likes and Layover will read it.

This changes what an existing Copilot factory does: its `fuel_usd` now refuses work, and so does
its Reserve.

**Why the Reserve is checked from history, at dispatch, and nothing more yet.** The Reserve was
configured, validated and drawn on the dashboard, and nothing refused work when it ran out — the
Tower never consulted it. Pricing Copilot runs made that impossible to leave: a rail an operator
can now watch fill up has to do something when it is full. So before a run starts, after
everything about the flight itself and before it counts against the chain's run cap, the Tower adds
up the measured spend in history over the Reserve's rolling window and refuses at the cap.

History rather than an in-memory tally, because history is where every finished run's cost already
lives, it survives a restart, and it is what the dashboard's Reserve card reads — so the rail and
the figure an operator sees cannot disagree. A refusal is written into history as a `halted` run
whose detail says how much was spent and when the window frees room: a scheduled trigger refused
before it starts leaves no other trace, and "nothing is running" must not look the same as "nothing
is allowed to run". It is recorded as a measured zero rather than `unreported`, because it cost
nothing and that is certain. If history cannot be read the check fails open, as history does
everywhere else; a factory that stopped because a log file was momentarily unreadable would have
turned an observability problem into an outage.

Enforcing the Reserve also enforces its documented default, $100 in any rolling 24 hours, on a
factory that never wrote `[reserve]`; `fuel_usd = 0` is how to say unlimited. Still not built from
the first-release design: the banner, the warning at 80%, the automatically raised help request,
and a cap raised by editing `layover.toml` without restarting the Tower — it reads its
configuration once. Nor does the check see runs still in flight (risk 15).

**Why the run cap is not the same as a cost rail.** Fuel depends on runners voluntarily reporting
cost and not all of them do, so the deterministic run cap holds when Fuel cannot. What changed is
that the gap is now *countable* rather than a boolean: an itinerary records how many of its runs
went unmetered, so "three of forty" and "all forty" are distinguishable. The first is a gap; the
second means the cost rail is not running at all.

**Why Fuel is required from the first runnable release rather than deferred.** Hops was originally assumed to be the
anti-fork-bomb rail. It is not. A hop is spent per flight and branches inherit the remaining
count, so Hops caps how *deep* a chain runs and says nothing about how *wide* it spreads — a
branching factor of 3 at `max_hops = 8` permits thousands of real, paid CLI invocations from a
single trigger. Only a shared per-itinerary budget bounds that, so Fuel moved from "later" to
"required". The follow-on consequence is that Fuel may not depend on runners voluntarily
reporting cost; it needs a deterministic fallback, or the single breadth rail can vanish silently
while still appearing to be enforced.
