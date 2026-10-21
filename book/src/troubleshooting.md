# Troubleshooting

Find the message you are looking at. Most of them come from one of three places: what
`layover serve` or another command prints, a run's or chain's detail on the
[dashboard](./dashboard.md), or [`layover doctor`](./cli.md#doctor), which reads a factory's history
and reports the failures that are otherwise quiet — run it first when something seems wrong and
nothing says so.

```sh
layover doctor                      # the last 7 days; --window last_24h or all_time
```

## Before anything runs

**`layover validate` refuses the factory.** It names the key and the problem. Unknown keys are
refused rather than ignored, so a typo fails here and not three agents deep. With `--strict`, a
warning fails too; [Configuration](./configuration.md) has every key.

**`another Tower is already serving this factory`.** One Tower serves a factory at a time, because
two would race for its queue. Stop the other with `layover stop --config <its layover.toml>`, or
find it in the dashboard it serves. The lock goes with the process, so a Tower that crashed leaves
nothing behind to delete.

**`could not listen on 127.0.0.1:7878: …`.** Something else has the port — often another factory's
Tower. Give this one another with `layover serve --addr 127.0.0.1:7880`, or set
`[layover] http_addr` so it always uses one.

**The dashboard answers `401`.** It needs the token in the address `layover serve` printed. A token
is minted each time a Tower starts — an update's new Tower keeps the old one's — so after a restart
a bookmarked page needs the new address once; the page keeps the token in a cookie from then on.

## A run fails as soon as it starts

**`program not found`** (Windows) or **`No such file or directory (os error 2)`.** The Tower could
not find the CLI on the `PATH` it was started with. A [preset](./runners.md#what-a-preset-supplies)
starts `copilot` or `claude` by its plain name, which on Windows finds only an `.exe`: an npm
install provides a `copilot.cmd` shim instead. Install the native build, or write the runner's
`command` out naming the shim. A Tower started at logon has the `PATH` of the logon, not of your
shell; see [Install](./install.md#agent-clis).

**`` `NAME` is named in `env_from` but is not set; … ``.** A variable the factory forwards to its
runs is not in the Tower's environment — a credential, or the CLI's
[home](./runners.md#a-cli-home-of-the-factorys-own). Set it where the Tower starts: the script that
runs `layover serve`, or, for a Tower started with the computer, where the service reads its
environment ([Install](./install.md#starting-with-the-computer)).

**The run's transcript says the CLI is not signed in.** The CLI reads its sign-in from its home. A
factory that points `COPILOT_HOME`, `CLAUDE_CONFIG_DIR` or `CODEX_HOME` somewhere of its own needs
signing in there once, with the variable set: `copilot login`, `codex login`, or `claude` and its
`/login`. Or forward a token the CLI reads, such as Copilot CLI's `GH_TOKEN`, with `env_from`.

**A Codex run fails, complaining about `-c`.** Codex cannot yet be told where Layover's MCP
endpoint is; see [The tools an agent has](./tools.md#how-a-run-reaches-them).

## A chain stopped

A chain's state on the [Chains](./dashboard-chains.md#chains) view says which of these it is, and
its detail says why.

**`failed`: `` `coder` failed, and nothing came after it: exited with code 1: … ``.** The run that
would have sent the work on did not finish, and nothing is queued after it. When the detail says
**the model service failed** (`http_400`, `transport_error`, …), the service ended the run rather
than the work: it is restarted after a back-off where the agent's `recovery` is `automatic`, up to
`max_recovery_attempts`, and the detail says why when it was not — see
[Recovery](./recovery.md#when-the-model-service-fails). Any other failure is the run's own, and is
never restarted. **Continue…** on the chain's page starts the workflow again with that chain's
flags.

**`stalled`: `` `publisher` never woke: nothing live could still deliver reviewer ``.** A joined
agent was waiting for a verdict that can no longer come. `doctor` reports
`N chain(s) stalled, stranding M flight(s)`.

**`waiting for you`.** The chain's last run filed a fatal help request and stopped. **Reply…**
answers it, and the work continues in a new chain with the same flags. `doctor` reports
`N open help request(s), M of which stopped the work`.

**`halted`.** A Ground Stop caught the chain, or the Reserve refused its next run. `doctor` reports
`a Ground Stop is engaged` — release it from the dashboard's header, or delete
`.layover/ground-stop` — or `the Reserve refused N run(s)`; see [Cost](./cost.md).

## Nothing seems to happen

**``scheduled pipeline `x` has not run in this window``.** A schedule that never fires looks exactly
like one that is working. Check that a Tower is serving the factory, that its workflow is not
switched off (`N workflow(s) switched off: …`), and that the schedule fires more often than the
window `doctor` looks at.

**``N flight(s) have been queued longer than `timeout_sec` with only N of M runs alive``.** Work is
waiting with a slot free: usually no Tower is running the queue. Start one with `layover serve`, or
drain the queue once with `layover run`.

**``N layover(s) waiting, and no pipeline declares `resumes = true` ``.** An agent set work down
meaning to come back to it, and nothing will collect it; see
[Pipelines](./pipelines.md).

## Cost and time

**Spend reads *not reported*, or `N of M run(s) reported no cost`.** The runner does not print the
output Layover reads a cost from. A preset supplies it; a `command` written out needs
`--output-format json` for Copilot CLI or `stream-json` for Claude Code, and Codex needs a
[rate card](./cost.md#rate-cards). See [Runners](./runners.md#the-output-that-says-what-a-run-cost).
Until it is fixed, Fuel and the Reserve cannot bind those runs; the run cap still does.

**``N run(s) outlived `timeout_sec` and were killed``.** A wedged run repeats identically, so the
same run will wedge again. Raise the timeout only if the work genuinely takes that long.

## Updating

**`Provenance not verified: the GitHub CLI, gh, is not installed`**, or **`gh is not signed in`.**
A release is installed only once `gh` has checked who built it. Install
[`gh`](https://cli.github.com) and run `gh auth login`, or, by your own choice,
`layover update stage --trust-checksum-only`.

**An unattended install does not happen.** The Tower says why in its output, once: a major release,
a cool-off not yet passed, outside the `window`, or a new warning against the factory. Everything
it will not take is still offered, and `layover update apply` installs it. See
[Updating Layover](./updating.md#installing-unattended).

**A release was rolled back.** The new version did not prove itself, so the old one was put back
and that version skipped. `layover update log` says what happened, and `doctor` mentions a rollback
for thirty days.
