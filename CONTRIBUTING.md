# Contributing

Contributions are welcome, from humans and from agents. The rules are the same for both, and they
live in [`AGENTS.md`](AGENTS.md) — read it before a first change. This page is the short version.

## The gate

```sh
cargo xtask verify
```

That is the only definition of done. It checks generated-code freshness, formatting, Clippy with
warnings denied, the documentation, the tests, and the doc build. **CI runs this exact command and
nothing else**, so a local pass really is a CI pass.

It matters more than any process around it. A lights-out factory does not stall because an agent
cannot write code; it stalls because nobody — agent or human — can tell whether the code is
correct without asking someone else. One command with a binary verdict means a first-time
contributor knows a change is acceptable before opening a pull request, rather than from a review
comment three days later — and it is what lets this repository be worked on unattended, which it
is.

`xtask` is not a tool you install — it is a crate in this repository (`xtask/`), so
`cargo xtask <task>` just builds and runs it. `cargo xtask` on its own lists the tasks.

If it fails, the output names the step. The two that surprise people:

| Step | What it means |
|---|---|
| `generated` | `api/openapi.yaml` changed and `crates/layover-http/src/generated.rs` did not. Run `cargo xtask generate-api`. |
| `docs` | A link or `#anchor` in a Markdown file no longer resolves; a file in `docs/` is missing from `book/src/SUMMARY.md`; or a command, flag, `layover.toml` key or endpoint is missing from its reference page. The message names which. |

## Getting set up

You need a Rust toolchain and nothing else. The version is pinned in
[`rust-toolchain.toml`](rust-toolchain.toml); rustup reads it automatically, so the right compiler
is used without you choosing one. That pinned version is also the minimum supported — it is the
only one tested.

```sh
git clone https://github.com/KotkaZ/layover-project
cd layover-project
cargo xtask verify
```

Development happens on Windows and CI runs Linux, so both are exercised. If you hit something
platform-specific, say so in the issue — that is useful information rather than noise.

## The crates

| Crate | What it is |
|---|---|
| `layover-core` | Config, agents, routes, pipelines, prompts, the route graph, validation, itinerary accounting, barriers |
| `layover-http` | The HTTP surface, **generated** from [`api/openapi.yaml`](api/openapi.yaml) |
| `layover-store` | On-disk history and journal: day-segmented JSON Lines, 90-day retention |
| `layover-dashboard` | The monitoring page and the API implementation behind it |
| `layover-tower` | The supervisor — the only crate that starts a process |
| `layover-mcp` | The MCP surface agents talk to Layover through |
| `layover-update` | Finding newer releases and staging them: the manifest, the download, its checksum and `gh`-verified provenance, the gate against a factory |
| `layover-cli` | The `layover` binary |

`AGENTS.md` says more about each, under *Where things live*.

## The documentation site

The book in `book/` is published to GitHub Pages: the latest release's at the root, `main`'s at
`/dev/`. Its design pages are written from `docs/` rather than kept twice, so building it locally
takes one step first:

```sh
cargo install mdbook --version =0.4.52 --locked
cargo install mdbook-mermaid --version =0.15.0 --locked
mdbook-mermaid install book
cargo xtask book          # writes book/src/design/ from docs/; never commit it
mdbook serve book --open
```

Edit `docs/`, not `book/src/design/`. Releasing is described in [`RELEASING.md`](RELEASING.md).

## What will get a change rejected

- **Weakening a safety rail.** Hops, Fuel, the Reserve, the run cap and Ground Stop are
  load-bearing. Changing what they permit is a design decision, not an implementation detail.
- **Editing generated code.** `crates/layover-http/src/generated.rs` is produced from
  `api/openapi.yaml`. Edit the specification.
- **Leaving the documentation behind.** `AGENTS.md` carries a table of what to update when. A
  stale document is worse than a missing one, because somebody will trust it.
- **A claim without a test.** Acceptance criteria have to be executable. If you cannot write a
  test that fails before the change and passes after, the change is not ready.
- **Secrets in configuration.** Credentials reach child CLIs through the environment. Never a
  committed file.

## Commits and pull requests

[Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`,
`refactor:`, `test:`, `chore:`.

Explain *why* in the commit message. The code already says what it does, and this repository is
read by people and agents starting cold who cannot recover intent from a diff.

Small, complete changes are easier to review than large ones. A pull request that does one thing
and carries its tests and its documentation will move quickly.

## Where to start

- **[`docs/decisions.md`](docs/decisions.md)** ends with the questions still open. They need a maintainer decision
  rather than a patch — but they are open to argument, and a well-reasoned case in an issue is a
  real contribution.
- **`docs/risks.md`** records known hazards, and says which are mitigated and which nobody has
  mitigated yet.
- **The examples** in `examples/` are exercised by tests. A new one that demonstrates a shape the
  others do not is welcome.

If you are unsure whether something would be accepted, open an issue before writing it. That is
cheaper for both of us than a rejected pull request.

## Reporting a vulnerability

Not here — see [`SECURITY.md`](SECURITY.md).
