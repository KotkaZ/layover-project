# Contributing

Contributions are welcome, from humans and from agents. The rules are the same for both, and they
live in [`AGENTS.md`](AGENTS.md) — read it before a first change. This page is the short version.

## The gate

```sh
cargo xtask verify
```

That is the only definition of done. It checks generated-code freshness, formatting, Clippy with
warnings denied, the documentation links, the tests, and the doc build. **CI runs this exact
command and nothing else**, so a local pass really is a CI pass.

`xtask` is not a tool you install — it is a crate in this repository (`xtask/`), so
`cargo xtask <task>` just builds and runs it. `cargo xtask` on its own lists the tasks.

If it fails, the output names the step. The two that surprise people:

| Step | What it means |
|---|---|
| `generated` | `api/openapi.yaml` changed and `crates/layover-http/src/generated.rs` did not. Run `cargo xtask generate-api`. |
| `docs` | A link in a markdown file no longer resolves, or an example path moved. |

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

- **`docs/decisions.md`** has an open-questions section. The ones under *Run bootstrap* block
  implementation and need a maintainer decision rather than a patch — but they are open to
  argument, and a well-reasoned case in an issue is a real contribution.
- **`docs/risks.md`** records known hazards that nobody has mitigated.
- **The examples** in `examples/` are exercised by tests. A new one that demonstrates a shape the
  others do not is welcome.

If you are unsure whether something would be accepted, open an issue before writing it. That is
cheaper for both of us than a rejected pull request.

## Reporting a vulnerability

Not here — see [`SECURITY.md`](SECURITY.md).
