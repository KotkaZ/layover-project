# Learnings and help requests

Part of the [decision log](../decisions.md): learnings that apply at once and lapse, help requests and the replies to them.

**Why learnings apply immediately instead of waiting for approval.** The obvious design puts a
human between a proposal and its use. An earlier agent factory built exactly that — proposal format,
duplicate detection, impact ratings, a review endpoint, a dashboard queue — and after 22 days of
real operation held 88 learnings, every one still pending, none ever approved. Since only approved
learnings were injected, not one had ever reached a run: everything was built except the step that
creates the value. That is not a discipline failure but an incentive one. Approving buys a diffuse
future benefit, rejecting buys nothing, and ignoring costs nothing today, so a gate whose default
action is free gets defaulted forever. A learning here applies at once and expires after twenty of
its agent's runs, so a wrong one decays rather than compounding, and review happens by exception.
That is only defensible because run history records which learnings were live for each run, which
makes "what was it told when it did that?" answerable and revocation a single press.

**Why an echo does not count as a rediscovery.** Confirmation comes from a learning being
independently arrived at three times, which is evidence — unlike the `impact` rating, which is the
agent's own claim about its own work and therefore decides nothing. The trap is that showing a
learning to an agent contaminates the signal: repeating advice you were just given proves nothing,
and counting it would let a single fluke confirm itself within three runs. So duplicates are
suppressed while a learning is active — the same behaviour the earlier factory needed, for the
opposite reason — and only a proposal arriving while the learning has lapsed increments the count.

**Why sameness is decided by containment rather than overlap.** Whether two sentences say the same
thing decides whether rediscovery is ever recognised, and it fails silently in both directions: too
strict and nothing is ever confirmed while the mechanism appears to work, too loose and two
insights merge and one is lost without trace. Word overlap is the wrong measure because real
learnings share sentence frames — "the workspace needs careful handling before publishing" and "the
manifest needs careful handling before publishing" overlap five words out of seven while being
different claims. Containment discriminates correctly: a rediscovery with an added clause is a
superset, while two different insights each carry a word the other lacks. A minimum length stops a
two-word learning matching everything, and a ceiling on elaboration keeps a substantially more
specific claim separate, which is what a refinement is.

**Why help requests are pruned and learnings are not.** Help requests are events: each happened at
a moment, is answered or not, and stops mattering. They are segmented by day and expire on the same
ninety-day horizon as run history. Learnings are state, rewritten as they are rediscovered or
revoked, and exempt from retention entirely — a confirmed learning that expired for being ninety
days old would be the one thing in the system that got worse the longer it was right.

**Why an unknown help category is refused rather than filed as `other`.** The category is optional,
because an agent that asks for help without classifying it has still asked, and refusing would
lose the one message meant to reach a person. But a value that is *not* a category is a mistake
somebody should see: filed as `other`, it would vanish from the dashboard's filter — the thing the
category exists for — and nobody would learn that the prompt names a category Layover does not
have. The refusal lists the six, so the agent's second attempt is right.

**Why answering a help request starts a new chain.** A fatal request ends its run, and with it the
chain, by design: an agent that cannot go on should stop rather than guess. So the answer cannot
resume anything — nothing is running, and resuming a process is settled against everywhere else.
It is a new run of the agent that asked, priced like a resumed layover: a fresh budget, because the
chain that asked is over and its Hops and Fuel were sized for the work it did. What carries over is
how the work was asked for — workflow, flags and routes — taken from the request, which records
them from the asking run's own session, never from what the agent says. The two chains are linked
both ways, through the answer recorded on the request and the link recorded on the new chain's
runs, so neither looks like it came from nowhere.

**Why a reply never takes the workflow's defaults.** The defaults are the safe choice for work
nobody has thought about. An answer is the opposite: someone triggered this chain with chosen
flags, and continuing it with the defaults silently downgrades it — a chain allowed to publish
becomes one that may not, and the only sign is a pull request that never appears. So a request
filed before requests recorded their chain's flags, which cannot know them, makes the person
replying say what they were, and is refused until they do. Decided with the maintainer on
1 October 2026, as was the next one.

**Why a reply records a name it is given.** Layover has one token per Tower and no accounts, so it
cannot know who answered. A name the person types, remembered by their browser, is the honest
record; failing that, the operating-system account the dashboard runs as, which on a factory one
person runs is that person.

**Why `awaiting_human` is a state of its own.** It is the second state, after `stalled`, that a list
of runs cannot show: every run ended cleanly and nothing is queued, and still the work is not done.
It applies only while the request the chain's *last* run filed is open and fatal — a request an
earlier run filed, which the chain went on past, does not hold it — and resolving the request
without an answer makes the chain `finished`, because that is a person deciding nothing more is
needed.

**Why a lapsed learning is deleted after a grace period the factory sets.** Lapsed learnings were
kept for ever, because a proposal matching one is a rediscovery and three of those confirm it. A
real factory lapsed 108 in twelve days — 86 of them from one sweep — and rediscovered one, twice, so
the Learnings tab became a list of what no longer applied (the maintainer, 2026-10-12, asked for
them deleted). That one rediscovery is the case for not deleting at once. Deleting one the moment it
lapses would remove the only path to confirmation that needs no person, so it is kept for
`[learnings] keep_lapsed_days` and then deleted. The period is the factory's to set, also the
maintainer's choice: how often a true insight comes round again depends on how often its agent runs,
which only the factory knows, and `0` gives the immediate deletion first asked for to a factory that
would rather rely on **Keep**. Fourteen days is the default — two weeks for a daily agent to arrive
at something true again, about 125 lapsed learnings kept at the rate above. The clock counts from
when the learning lapsed, now recorded; one that lapsed before that counts from when it was last
proposed, which is earlier, so it never stays longer than the period says. Rejected learnings are
not deleted: they are what stops an agent re-proposing what a person refused. Deletion happens after
every run and when the Tower starts — not from a dashboard that is only watching, which would
rewrite a file the Tower in another process is writing.

**Why the numbers a learning lives by are the factory's, and two things about learnings are not.**
Twenty runs, three rediscoveries, 400 characters and 25 a run were chosen for one factory's pace;
asked to make them configurable with the deletion period (the maintainer, 2026-10-12), all four
moved into `[learnings]` with those values as defaults. How many runs tell a fluke from a fact
depends on how often the agent runs, which only the factory knows. Each is checked for the one value
that would stop learnings working with nothing failing — zero — and otherwise taken as written:
raising one widens what a wrong learning can do (risk 19 says so), but the factory that raises it is
the one that pays for it. Two things stayed fixed. Whether two learnings say the same thing decides
whether a rediscovery is recognised, and it fails silently in both directions — a threshold set
wrong confirms nothing or merges two insights into one, and nobody would see either. And what a
learning may not say is a screen against injected instructions: a knob on it is a knob an attacker
would want turned.
