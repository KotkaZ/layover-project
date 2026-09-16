# Install

Layover is a single binary called `layover`. It needs no runtime — not Rust, not Node.

> **Nothing is released yet.** The commands below work from the first `v*` tag onwards; until
> then, use [from source](#from-source). See [Cutting a release](#cutting-a-release).

## macOS and Linux

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-installer.sh | sh
```

## Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-installer.ps1 | iex"
```

Both installers pick the right build for your platform, verify its SHA-256 against the checksum
published beside it, unpack it and put `layover` on your `PATH`.

If piping a script from the internet into a shell makes you uncomfortable — reasonably — download
it first and read it. It is about a hundred lines.

## With npm

Worth knowing about, because if you are using Layover you almost certainly already have Node: the
agent CLIs it supervises all ship as npm packages.

```sh
npm i -g https://github.com/KotkaZ/layover-project/releases/latest/download/layover-cli-npm-package.tar.gz
```

The package downloads the right prebuilt binary for your platform; nothing is compiled. It is not
on the public registry yet, so the tarball URL is the install path for now.

## Manual download

Every release attaches an archive per platform with a `.sha256` beside it:

| Platform | Archive |
|---|---|
| Linux x86-64 | `layover-cli-x86_64-unknown-linux-gnu.tar.xz` |
| Linux ARM64 | `layover-cli-aarch64-unknown-linux-gnu.tar.xz` |
| macOS Intel | `layover-cli-x86_64-apple-darwin.tar.xz` |
| macOS Apple silicon | `layover-cli-aarch64-apple-darwin.tar.xz` |
| Windows x86-64 | `layover-cli-x86_64-pc-windows-msvc.zip` |

Unpack it and put `layover` somewhere on your `PATH`. A `sha256.sum` covering every artifact is
attached to the release too.

## With Cargo

If you already have a Rust toolchain:

```sh
cargo install layover-cli
```

The binary is called `layover`, not `layover-cli`.

## From source

```sh
git clone https://github.com/KotkaZ/layover-project
cd layover-project
cargo install --path crates/layover-cli
```

## Checking it worked

```sh
layover --version
```

## Agent CLIs

Layover supervises other tools; it does not replace them. Install whichever runners your factory
names, and make sure each works on its own before pointing Layover at it:

| Runner | Install | Check |
|---|---|---|
| Claude Code | `npm i -g @anthropic-ai/claude-code` | `claude --version` |
| GitHub Copilot CLI | `npm i -g @github/copilot` | `copilot --version` |
| OpenAI Codex CLI | `npm i -g @openai/codex` | `codex --version` |

Credentials reach child CLIs through the environment. **Never put an API key in `layover.toml`** —
it is a file people commit.

## Starting with the computer

A lights-out factory that stops at every reboot is not lights-out.

```sh
layover autostart                 # writes the file
layover autostart --show          # print it instead, to read first
```

That generates your platform's own artefact — a Scheduled Task on Windows, a launchd agent on
macOS, a systemd user unit on Linux — and prints the single command that registers it. It does
**not** register it for you: that touches the machine, and you should see what is being installed.

It also refuses to write anything if the factory does not load, because a service that fails at
every logon is worse than no service.

All three run as **you**, never elevated and never machine-wide. The Tower spawns agents that use
your provider credentials, your git identity and your workspace; a system service would have none
of them, or would run as root with all of them.

## Verify your setup

```sh
layover validate --config layover.toml --strict
```

This exits non-zero if anything would stop the factory starting. It is worth running in CI over
your factory definition: an unattended factory that discovers a typo three agents deep has
already spent money to find out.

## Why not Docker

Layover spawns agent CLIs as child processes, gives them a git worktree of *your* workspace, and
relies on *your* provider credentials and MCP configuration. A container would have to be handed
all three, at which point it has your filesystem and your secrets and has bought you nothing. It
is a local-first supervisor; run it locally.

## Cutting a release

Releases are built by [`dist`](https://opensource.axo.dev/cargo-dist/), configured in
`dist-workspace.toml`. Tagging is the whole process:

```sh
git tag v0.2.0
git push origin v0.2.0
```

That builds all five targets, generates the installers, checksums everything and publishes a
GitHub Release. `.github/workflows/release.yml` is **generated** — change `dist-workspace.toml`
and run `dist generate`, never edit the workflow by hand.

Check the configuration without releasing anything:

```sh
dist plan
```
