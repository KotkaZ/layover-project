# layover

**Run a lights-out agent factory.**

Layover does not call LLMs. It is a *supervisor*: it spawns headless agent CLIs (`claude -p`,
`copilot`, `codex exec`), gives them a way to reach each other, persists what they learn, and
stops them from running away.

Full documentation: <https://kotkaz.github.io/layover-project/>

## Install

Layover is a single binary. From crates.io, if you have a Rust toolchain:

```sh
cargo install layover-cli
```

...or, with no Rust toolchain, Homebrew on macOS and Linux:

```sh
brew install KotkaZ/tap/layover
```

...or a shell one-liner, a PowerShell one-liner, or npm — see the
[install guide](https://kotkaz.github.io/layover-project/install.html).

The binary is called `layover`.

## Use

```sh
layover validate --config layover.toml     # check a factory before it runs
layover explain                            # what can trigger what, and what talks to what
layover graph --pipeline nightly           # one workflow's route map, as Mermaid
layover prompt tester --flag run_e2e=true  # what an agent would actually be told
layover serve                              # run the factory and serve the dashboard
layover stop                               # stop it once what it is running has finished
layover doctor                             # report anything a person should look at
layover update check                       # is there a newer Layover, and what does it change
layover update apply                       # install it between runs, and restart on it
layover update rollback                    # put the previous version back
layover about                              # what this binary is
layover autostart --show                   # the file that starts Layover at logon
```

`layover validate` exits non-zero when anything would block startup. Run it in CI over your
factory definition: an unattended factory that discovers a typo three agents deep has already
spent money to find out.

`layover doctor` exits non-zero when a factory's recorded history contains something that would
fail an unattended run — a stalled chain, a schedule that never fired, a Ground Stop left engaged,
or a quarter of its runs reporting no cost, with the runner to fix and what to add.

Give a factory a CLI home of its own — `COPILOT_HOME`, `CLAUDE_CONFIG_DIR` or `CODEX_HOME`,
forwarded with `env_from` — or its runs fill your own session history and load your personal
plugins and MCP servers. See
[the CLI's home](https://kotkaz.github.io/layover-project/runners.html#a-cli-home-of-the-factorys-own).

## Status

What Layover does today, and what it does not do yet, is in the book's
[Status](https://kotkaz.github.io/layover-project/#status); why it is built as it is, in
[the decision log](https://github.com/KotkaZ/layover-project/blob/main/docs/decisions.md).

## License

[Apache-2.0](https://github.com/KotkaZ/layover-project/blob/main/LICENSE)
