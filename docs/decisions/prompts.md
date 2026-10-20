# Prompts, flags and what a run is told

Part of the [decision log](../decisions.md): conditional prompts, the sender, flags and names that travel with a chain, and reports.

**Why prompt composition is conditional rather than templated.** Prompts wanted to vary — a tester
that sometimes runs a remote suite is the same agent with one extra paragraph — and the obvious
answer is string interpolation. Booleans chosen at trigger time are deliberately less powerful:
every possible prompt is a file somebody can read and review, and `layover prompt` can render any
of them exactly as a run would receive it. A templating language would make prompts programs, and
the thing an agent is told would stop being reviewable. The rails around it all exist because the
failure mode is silent: an undeclared flag is an error rather than false, because treating it as
false would let a typo delete a section of instructions without anyone noticing.

**Why a run is told who sent its flight, and in which words.** The prompt guide and the routing
notes told authors that sender identity is how an agent tells its inputs apart, but the payload
held only the body, so factories wrote `FROM <agent>` into every body by hand — an identity the
sender asserts, which is the thing §4.2 of the architecture exists to rule out. The Tower already
records who sent each flight, so the payload states it above the body. It says what *kind* of
sender it was, because "fix the retry policy" means one thing from a reviewer and another from a
clock nobody is watching; that needed `Origin` to stop recording a schedule and a resumed layover as
a person. A released join states nothing there: its body already labels each flight it folds
together, and naming one sender above them would be wrong about the rest.

**Why prompt flags are checked per entry point rather than per factory.** A run carries the flags
of the one pipeline that triggered it, never the union of every pipeline in the factory. Checking
a prompt against that union looks equivalent and is not: a second pipeline that reaches the same
agent without declaring the flag passes validation and then fails at composition time, hours
later, with nobody watching. So validation walks reachability from *each* entry point and requires
every flag a reachable prompt tests to be declared *there*. The same rule covers a bare
`entry = true` agent, which supplies no flags at all — any conditional prompt downstream of one is
unreachable in practice, and saying so at load time is the whole point.

**Why a chain's flags travel with its work.** A flag is a choice somebody makes when they trigger
work, and it was being honoured for nothing: the trigger stored it, and every run then composed
its prompt from the pipeline's defaults, so `layover prompt --flag` previewed text no run received.
Remembering the flags per itinerary in the Tower fixed the first run and nothing after a restart,
because that memory starts empty. So each queued flight carries its chain's pipeline and flags,
the way the first flight always did, and the Tower records them from whichever flight it sees
first. The values come from the Tower's record of the run — the token's session — and never from
the agent, because an agent that could set a flag could switch on the section of its instructions
that lets it publish.

**Why a spawned chain belongs to the pipeline that spawned it.** A spawn edge gives a chain fresh
Hops, Fuel and a run cap; it was also giving it no pipeline, so its prompts were composed from
every pipeline's defaults merged — the last declaration winning on a clash — and its runs and cost
belonged to no workflow. The first contradicted the operator, the second contradicted the rule
that workflow membership crosses spawn edges. A spawned chain now inherits both the flags and the
pipeline of the chain that spawned it, and prompt-flag validation follows spawn edges for the same
reason: the spawned agent is composed from the spawning entry point's declarations.

**Why a resumed layover takes its flag values from the chain that booked it.** The follow-up is the
same work, and the choices about that work — run the end-to-end suite, keep the pull request a
draft — were made when it was triggered; resuming from the resuming pipeline's defaults undid them
days later with nobody watching. But the *set* of flags stays the resuming pipeline's, because
validation proves the prompts reachable from there test only what it declares; composing from the
booking chain's set instead could fail at runtime on a factory that passed `validate`. So the
layover records the booking chain's flags, and the resumed run takes those values for the flags
the resuming pipeline declares and its defaults for the rest.

**Why an agent writes its own report rather than the Tower keeping the transcript.** A transcript
is not a report: it contains every approach the agent abandoned, so reading one to find out what
happened is slower than doing the work again. The Tower could store transcripts and separately ask
for a summary, but requiring the agent to state its own conclusion has a second effect worth more
than the storage — an agent that must write down what it concluded is an agent that has to decide
what it concluded. Reports are capped and truncated rather than rejected, because a report is the
only account of a run that has already happened and already cost money, and discarding it to punish
a formatting mistake loses the thing entirely. A trimmed report says it was trimmed, so a reader
knows to look further rather than assuming the agent stopped there.

**Why a chain's name and per-agent choices travel exactly as its flags do.** A name is how a person
finds *their* run of a workflow among three going at once, so it has to stay with everything that
run causes — a hand-off, a spawned review, a follow-up two days later, the continuation a reply
starts — or the second half of the work is unnamed. A per-agent choice is the same: asking for the
reviewer at `max` on one hard change means every review in that change. So both ride on the queued
flight, the Tower's record of the chain, the MCP session, the layover and the help request, the
path flags already proved, and an agent can neither read nor set them through a tool. A trigger may
choose a model, effort or context — values a runner already carries through a placeholder — and
never arguments, because arguments are how a runner denies things and the dashboard is not where
permissions are decided. A choice that could not take effect (an agent the workflow never runs, a
value with no placeholder) is refused, because accepted and ignored is the failure this project
keeps finding. A chain's own map is captioned from what its runs were recorded running at, rather
than from configuration, because configuration can change after a chain ran and the record cannot.

**Why `[flags]` is accepted everywhere but offered narrowly.** Every workflow that reaches a prompt
testing a flag must accept it, so a factory whose workflows share agents declared the same flags in
each — thirty declarations for ten flags in a production factory, one more copy to drift each time.
A factory-wide table had two possible meanings: the flag applies where some reachable prompt tests
it, or everywhere. Only "everywhere" can be decided from `layover.toml` alone, which is all the
parser has, so every workflow accepts every shared flag and a run is never composed short of one.
But a trigger window listing switches that change nothing in that workflow teaches people to ignore
switches, so what is *offered* — the window, `explain` — is narrowed by reading the prompts: a
workflow's own flags, and the shared ones its agents test. A workflow that repeats a shared flag
exactly is warned about rather than refused; one that gives it a different default is the
disagreement validation already warns about. The merge happens as the file is read, so nothing
downstream can forget to apply it.
