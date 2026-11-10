---
name: install-layover
description: Install Layover, the `layover` command that supervises headless agent CLIs (GitHub Copilot CLI, Claude Code, Codex) as an unattended agent factory, on Windows, macOS or Linux. Downloads the latest release, checks its SHA-256 and GitHub build provenance, installs it where Layover can later update itself, and checks the agent CLIs a factory drives. Use when asked to install, set up, reinstall or check an installation of Layover, or to get a machine ready to run a Layover factory.
license: Apache-2.0
---

# Install Layover

Layover is one binary, `layover`, with no runtime to install. These steps install the latest
release, prove the download is the file Layover's release workflow built, and put it in the Cargo
home's `bin` — where Layover's own installers put it, and where a factory can later replace it with
a newer release by itself.

Work through the steps in order and read each command's output before the next. **If a checksum or
provenance check fails, stop and tell the user. Never retry around it, skip it, or fall back to an
unchecked install.**

## 1. Is Layover already here?

```sh
layover about --json
```

| What happens | What it means | Do |
|---|---|---|
| JSON with a `"version"` | Layover 1.15.0 or later is installed | **Do not install over it**: a factory may be running from that file. For a newer release, run `layover update check`, then `layover update apply` in the factory's directory (beside its `layover.toml`), which drains the factory, so that nothing new starts and running work has time to finish, before it replaces anything. Report the version and stop, unless the user asked for something else. |
| It fails, but `layover --version` prints `layover 1.x` | A Layover older than 1.15.0, from before it could update itself | Ask the user to stop any factory running from it, then reinstall it the way it was installed if they know (`brew upgrade KotkaZ/tap/layover`, `cargo install layover-cli`), or as below. |
| Anything else | A different program named `layover`: an unrelated SSH-tunnelling crate installs one | Show the user where it is (`command -v layover`, or `Get-Command layover` in PowerShell) and ask before going on, since one will shadow the other on `PATH`. |
| Command not found | Not installed | Go on. |

## 2. Choose how to install

Use the checked download in step 3 unless the user asked for another way.

| Way | When | Know this |
|---|---|---|
| **Checked download** (step 3) | The default, on every platform | Checks the SHA-256 always, and provenance when `gh` is signed in; stops on a mismatch. A factory can update it in place later. |
| `brew install KotkaZ/tap/layover` | macOS or Linux, when the user wants Homebrew to manage it | Homebrew checks the formula's checksum. New releases come from `brew upgrade KotkaZ/tap/layover`; Layover leaves a Homebrew install to Homebrew. |
| `cargo install layover-cli` | A Rust toolchain is present and the user wants it built from source, or no prebuilt build runs here | **Never `cargo install layover`.** That crate is an unrelated SSH-tunnelling tool whose binary is also called `layover`, so the mistake is silent. |
| The one-line installers, or npm | Only when the user names them | The PowerShell installer and the npm package check no checksum; the shell installer checks one only when `sha256sum` exists. Say so first. Their commands are in [the install guide](https://kotkaz.github.io/layover-project/install.html). |

Do not pipe a downloaded script into a shell unless the user asked for that installer by name.

## 3. The checked download

### Pick the build

| `uname -s`, `uname -m` | `TARGET` |
|---|---|
| `Linux`, `x86_64` | `x86_64-unknown-linux-gnu` |
| `Linux`, `aarch64` or `arm64` | `aarch64-unknown-linux-gnu` |
| `Darwin`, `arm64` | `aarch64-apple-darwin` |
| `Darwin`, `x86_64` | `x86_64-apple-darwin`, unless `sysctl -n hw.optional.arm64` prints `1`: then the shell runs under Rosetta on Apple silicon, so use `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc`, on x64 and on Windows 11 on Arm, which runs it under emulation |

There is no build for musl-based Linux such as Alpine, the BSDs, or 32-bit systems: use
`cargo install layover-cli` there.

### macOS and Linux

Run it as one command with `TARGET` from the table. It stops at the first failure, and removes what
it downloaded either way.

```sh
TARGET=x86_64-unknown-linux-gnu sh -eu <<'EOF'
FILE="layover-cli-$TARGET.tar.xz"
BASE=https://github.com/KotkaZ/layover-project/releases/latest/download
BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
curl -fsSLO "$BASE/$FILE"
curl -fsSLO "$BASE/$FILE.sha256"
if command -v sha256sum >/dev/null; then sha256sum -c "$FILE.sha256"; else shasum -a 256 -c "$FILE.sha256"; fi
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  gh attestation verify "$FILE" --repo KotkaZ/layover-project
else
  echo "PROVENANCE NOT CHECKED: gh is not installed or not signed in"
fi
tar -xf "$FILE"
mkdir -p "$BIN"
install -m 755 "layover-cli-$TARGET/layover" "$BIN/layover"
"$BIN/layover" about --json
EOF
```

If the last line fails with `GLIBC_2.xx not found`, this Linux's C library is older than the one the
build needs: delete the `layover` it installed and use `cargo install layover-cli` instead.
`sha256sum` may also warn that a line is improperly formatted: the `.sha256` file ends with a blank
line. The `OK` beside the archive's name is what counts.

### Windows

Run it as one command, in PowerShell 7 or Windows PowerShell 5.1:

```powershell
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'
  $file = 'layover-cli-x86_64-pc-windows-msvc.zip'
  $base = 'https://github.com/KotkaZ/layover-project/releases/latest/download'
  $bin  = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $HOME '.cargo\bin' }
  $work = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid())
  New-Item -ItemType Directory $work | Out-Null
  try {
    Invoke-WebRequest "$base/$file" -OutFile "$work\$file" -UseBasicParsing
    Invoke-WebRequest "$base/$file.sha256" -OutFile "$work\$file.sha256" -UseBasicParsing
    $expected = (Get-Content "$work\$file.sha256" -Raw).Split(' ')[0]
    $actual   = (Get-FileHash "$work\$file" -Algorithm SHA256).Hash
    if ($actual -ine $expected) { throw "checksum mismatch: got $actual, expected $expected" }
    $gh = Get-Command gh -ErrorAction SilentlyContinue
    if ($gh) { cmd /c 'gh auth status >nul 2>&1' }
    if ($gh -and $LASTEXITCODE -eq 0) {
      gh attestation verify "$work\$file" --repo KotkaZ/layover-project
      if ($LASTEXITCODE -ne 0) { throw 'provenance refused: do not install this file' }
    } else {
      Write-Warning 'PROVENANCE NOT CHECKED: gh is not installed or not signed in'
    }
    Expand-Archive "$work\$file" -DestinationPath "$work\unpacked"
    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item "$work\unpacked\layover.exe" (Join-Path $bin 'layover.exe') -Force
    & (Join-Path $bin 'layover.exe') about --json
  } finally {
    Remove-Item -Recurse -Force $work
  }
}
```

## 4. Put it on `PATH`

Run `layover about --json` in a **new** shell. With Rust installed, the Cargo home's `bin` is usually
on `PATH` already. If it is not, tell the user, and offer to add it. Do not edit a shell profile or
the user's `PATH` without asking.

- **macOS and Linux:** add `export PATH="$HOME/.cargo/bin:$PATH"` to the profile of the shell
  `echo $SHELL` names: `~/.zshrc` for zsh, macOS's default; `~/.bashrc` or `~/.profile` for bash.
- **Windows:** as Layover's own installer does, put it first on the user's `PATH`, keeping `%…%`
  entries unexpanded, and tell Windows so that programs started afterwards see it:

  ```powershell
  $bin = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $HOME '.cargo\bin' }
  $key = 'registry::HKEY_CURRENT_USER\Environment'
  $dirs = (Get-Item -LiteralPath $key).GetValue('Path', '', 'DoNotExpandEnvironmentNames') -split ';' -ne ''
  if ($bin -notin $dirs) {
    Set-ItemProperty -Type ExpandString -LiteralPath $key Path ((,$bin + $dirs) -join ';')
    $name = 'layover-' + [guid]::NewGuid()
    [Environment]::SetEnvironmentVariable($name, 'x', 'User')
    [Environment]::SetEnvironmentVariable($name, [NullString]::Value, 'User')
  }
  ```

A shell that is already open keeps its old `PATH` until it is restarted.

## 5. The agent CLIs, and `gh`

Layover runs agent CLIs; it does not include them. Check each one the user's factory will use:

| CLI | Check | Install |
|---|---|---|
| GitHub Copilot CLI | `copilot --version` | `npm i -g @github/copilot` |
| Claude Code | `claude --version` | `npm i -g @anthropic-ai/claude-code` |
| OpenAI Codex CLI | `codex --version` | `npm i -g @openai/codex` |

Install only the ones the user wants. Signing in to each is theirs to do.

On Windows, a runner preset starts its CLI by its plain name, which finds only an `.exe`. If
`Get-Command copilot` (or `claude`) is a `.cmd` or `.ps1` from npm, say so: a preset will not find
it. The fixes are in [Agent CLIs](https://kotkaz.github.io/layover-project/install.html#agent-clis).

`gh`, installed and signed in (`gh auth status`), is what lets a factory install new Layover
releases unattended: by default it installs only what `gh` has verified. Suggest it if it is
missing; a factory runs without it.

## 6. Report, and what comes next

Tell the user:

- the version `layover about --json` prints, and where the binary is;
- whether the checksum was checked, and whether provenance was. Say plainly if provenance was not;
- whether `layover` is on `PATH` for new shells, and anything they still have to do;
- which agent CLIs were found, and which still need installing or signing in.

Then point them on:

- [Your first factory](https://kotkaz.github.io/layover-project/first-factory.html): three agents
  in a loop, on Copilot CLI. If you have the `first-layover-factory` skill, offer to set one up with
  them now. Never create a factory inside a checkout of Layover's own source.
- `layover validate --strict` in the factory's directory, before anything runs.
- [Starting with the computer](https://kotkaz.github.io/layover-project/install.html#starting-with-the-computer):
  `layover autostart` writes what starts the factory at logon and prints the command that
  registers it; `--show` prints the file instead. Registering touches the machine, so leave that to
  the user unless they ask.
- API keys never go in `layover.toml`; agent CLIs get credentials from the environment. Give the
  factory [a CLI home of its own](https://kotkaz.github.io/layover-project/runners.html#a-cli-home-of-the-factorys-own),
  so its runs stay out of the user's own sessions.
