# The shape of the system

Part of the [decision log](../decisions.md): supervising CLIs, MCP, fresh runs, the Logbook, the generated HTTP server, licence, toolchain and documentation.

**Why supervise CLIs instead of calling LLMs directly.** The agent CLIs already solve tool use,
sandboxing, context management and provider auth. Reimplementing that would be the whole project.
Supervising them means Layover's scope stays orchestration, and it inherits improvements to those
CLIs for free. The cost is that agents become opaque processes, which is what forces §4.2.

**Why MCP is the control channel.** All three target CLIs speak MCP natively, so agents gain the
ability to message each other with no adapter code and no bespoke protocol. This also answered the
open interop question: Layover does not need to pick between A2A, ACP or a greenfield protocol,
because the control channel and the integration standard turned out to be the same problem.

**Why sending a message is what starts an agent.** Two operations — spawn and send — would need
two permission models over the same graph, and would allow the incoherent state of an agent
spawned with nothing to do. Unifying them makes the route map the single authority over both
communication and lifecycle.

**Why runs are fresh rather than resumed.** Resumed sessions grow without bound, make cost
unpredictable, and hide what an agent actually knows inside an opaque transcript. Fresh runs force
memory to be deliberate and inspectable. See §4.1.

**Why shared memory is Markdown rather than SQLite.** The Logbook is meant to be read by humans
during an incident and edited by hand when an agent records something wrong. A database would be
more robust and less useful. Write safety is recovered by serializing writes through the Tower
rather than by the storage format.

**Why workspace contention is only partly mediated.** Full locking or per-run worktrees for every
agent are real work and would delay proving the core concept. The decision was that read-only
agents get a worktree snapshot, which would make the common fan-out shape safe for free; two
concurrent read-write agents remain unsafe and the risk is recorded in
[`risks.md`](../risks.md#2-shared-workspace-contention) rather than forgotten. *The snapshot is not
built yet* — every agent runs in its `work_dir` — and what building it has to settle is open
question 8 below.

**Why the factory never targets Layover's own source.** It removes an entire class of hazard —
agents editing the supervisor that is running them — and makes it safe to give agents full write
access inside their workspace. The cost is losing the most persuasive dogfooding demo.

**Why the HTTP server is generated from a specification rather than described by one.** A
hand-written server with a hand-written OpenAPI document beside it has two sources of truth and no
force keeping them equal; the document rots first and quietly, because nothing breaks when it
does. Generating the server makes the document load-bearing: a drifted spec is a failing build,
not a stale page. The cost is a generator to maintain, and it is kept cheap by supporting only the
subset this API uses and erroring loudly on anything else — a generator that silently ignored
part of the specification would recreate exactly the problem it was meant to solve.

**Why Apache-2.0 alone rather than the Rust-typical `MIT OR Apache-2.0`.** Rust libraries dual
licence so that GPLv2-only projects can consume them, since Apache-2.0 is incompatible with
GPLv2. That reasoning is about libraries, and Layover is an application: people run the binary,
they do not link `layover-core` into something else. What Apache-2.0 adds over MIT is an explicit
patent grant, which is worth more here than compatibility with a licence nobody consuming a
supervisor is likely to be bound by. Revisit if `layover-core` is ever published as a crate meant
to be depended on — at that point the dual licence becomes the right default again.

**Why the milestone is named rather than numbered.** "v0.1" used to mean the release where a
factory first actually runs, while the crates were already at 0.8.0 — so the project appeared to
be both far behind and far ahead of itself, and a stranger could not tell which. Crate versions
now track what is built and the milestone has a name instead of a number. A goal spelled like a
version will be read as a version. The milestone was reached at 1.0, on 23 September 2026, and
from there the version follows semantic versioning.

**Why the declared MSRV is the pinned toolchain.** `rust-version` said 1.85 while
`rust-toolchain.toml` pinned 1.98.1, which is the only compiler CI ever runs — so the older
number was an untested claim, and thirteen minor versions is a long way to be wrong. The choice
was to test the claim or to stop making it. Testing it means a second CI job and a second
toolchain download on every push to protect users nobody has yet; stating the version that is
actually exercised costs nothing and is true. If somebody turns up needing an older compiler,
that is the moment to find out how far back it really builds.

**Why documentation upkeep is in `AGENTS.md` rather than in review.** `docs/` is normative and
hand-maintained, and an agent that trusts a stale document makes confident wrong changes. Review
catches that only if a human remembers to look. So the standing instruction is that every change
carries its documentation, `AGENTS.md` names which file goes with which kind of change, and
`verify` enforces the mechanical part — links that resolve, generated code that matches, examples
that still validate. No tool can check whether a paragraph is still true, which is precisely why
the instruction has to be standing rather than requested.

**Why the decision log is split by topic.** One file had grown past 1,400 lines, and a file an
agent's reader cannot take in whole is one it cannot reason about — the rule `AGENTS.md` sets at
about 500 lines. Splitting by date would have scattered one subject across files; by topic, the
reasoning a change needs is in one place. Within a file the entries keep the order they were made
in, and `decisions.md` keeps the index and the questions still open, so every link to it still
lands somewhere true.

**Why the design documents are written into the book rather than moved there.** `docs/` is where
agents read the design: `AGENTS.md`, code comments and the documents themselves point there.
Moving it into `book/src/` would move every one of those; copying it would make two sources of
truth. So `cargo xtask book` writes the book's design pages from `docs/` when the book is built,
rewriting the links that only work in the repository — to code, to GitHub at the commit published;
to a book page, within the book — and the copy is never committed. `verify` checks a link into
those pages against `docs/`, and that every file in `docs/` is listed in the book.

**Why the site shows the latest release, with `main` beside it.** The site was rebuilt from `main`
on every push, so it described what no release did yet — the pages about updating were live three
days before 1.15.0 shipped — with nothing to say so to a reader on the installed version. Its root
now follows the latest release, and `main` is published at `/dev/` with a banner on every page, so
a fix to the documentation is visible at once and reaches the root with the next release (decided
with the maintainer, 2026-10-20). The rebuild is triggered by the Release workflow finishing,
because a release published with the workflow's own token starts no other workflow.
