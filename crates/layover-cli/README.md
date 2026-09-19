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
layover serve                              # run the factory and serve the dashboard
layover doctor                             # report anything a person should look at
layover autostart --show                   # the file that starts Layover at logon
```

`layover validate` exits non-zero when anything would block startup. Run it in CI over your
factory definition: an unattended factory that discovers a typo three agents deep has already
spent money to find out.

`layover doctor` exits non-zero when a factory's recorded history contains something that would
fail an unattended run — a stalled chain, a schedule that never fired, a Ground Stop left engaged.

## Status

`layover serve` runs a factory unattended: it fires scheduled pipelines, spawns agent CLIs, hosts
the MCP endpoint they call back into, enforces the safety rails, and serves a dashboard over all
of it.

See [the decision log](https://github.com/KotkaZ/layover-project/blob/main/docs/decisions.md).

## License

[Apache-2.0](https://github.com/KotkaZ/layover-project/blob/main/LICENSE)
