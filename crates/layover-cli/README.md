# layover

**Run a lights-out agent factory.**

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs (`claude -p`,
`copilot`, `codex exec`), gives them a way to reach each other, persists what they learn, and
stops them from running away.

Full documentation: <https://kotkaz.github.io/layover-project/>

## Install

Layover is a single binary. No Rust toolchain needed.

```sh
cargo install layover-cli
```

...or a shell one-liner, a PowerShell one-liner, or npm — see the
[install guide](https://kotkaz.github.io/layover-project/install.html).

The binary is called `layover`.

## Use

```sh
layover validate --config layover.toml     # check a factory before it runs
layover explain                            # what can trigger what, and what talks to what
layover prompt tester --flag run_e2e=true  # what an agent would actually be told
```

`layover validate` exits non-zero when anything would block startup. Run it in CI over your
factory definition: an unattended factory that discovers a typo three agents deep has already
spent money to find out.

## Status

Early. What works today is everything before the first spawn — loading a factory definition,
checking it, and showing what it would do. Process supervision, the MCP server and the HTTP API
are not built yet.

See [the roadmap](https://github.com/KotkaZ/layover-project/blob/main/docs/roadmap.md).

## License

[Apache-2.0](https://github.com/KotkaZ/layover-project/blob/main/LICENSE)
