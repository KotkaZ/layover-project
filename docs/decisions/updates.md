# Updating Layover

Part of the [decision log](../decisions.md): looking for, verifying, staging and installing releases, and rolling them back.

**Why Layover looks for its own updates after all.** The first release decided against self-update
because a tool that rewrites its own binary while supervising running children is a category of bug
nobody needs, and the installers already handle it (see `first-release.md`). Both halves have
changed. A Tower can now stop without cutting anything off — it drains — so "while supervising
running children" is a condition that can be waited out rather than one that is always true. And the
installers handle installing, not hearing: a factory left unattended runs whatever it was installed
with, for weeks, through fixes made for faults it is having. The maintainer decided on 15 October
2026 to revisit it on those terms: Layover may update itself, never while a run is alive or a
rendezvous holds an arrival. It arrives in steps, each useful alone. The first only tells: the Tower
asks once a day whether a newer release exists, and the dashboard, `doctor` and
`layover update check` say what it changes. Downloading, verifying and applying come later, each
behind the step before it.

**Why telling is the default, and nothing more.** A factory that says nothing in `[update]` is told
about releases and changes nothing. Telling costs one request a day for a public file and cannot
break a factory; applying can, and an existing factory should not start doing it because its binary
was replaced. Off by default would be safer still and would leave the factories that most need a
fix — the ones nobody is watching — never hearing of it. `LAYOVER_UPDATE=off` turns every request
off on a machine whatever its factories say, for CI and for networks that must not reach out.

**Why discovery reads the download address, not the API.** `api.github.com` allows sixty
unauthenticated requests an hour per address, and behind a corporate NAT that budget belongs to the
whole building. The newest stable release's manifest is at
`releases/latest/download/dist-manifest.json`, a redirect github.com and its asset host serve without
that limit, and the manifest already says everything a person needs: the version, whether it is a
pre-release, the release notes with their Compatibility paragraph, and each platform's archive with
its SHA-256. The API is used only for what has no such address — the newest *pre-release* — and
takes `LAYOVER_GITHUB_TOKEN`, never `GITHUB_TOKEN`, which is usually a token meant for something
else. The upload time from the manifest's `Last-Modified` is the release's age, which a later step
needs for the cool-off before an unattended factory takes a release.

**Why what was found is kept per machine.** The binary being updated belongs to the installation,
which several factories may share. Kept per factory, two factories would each ask GitHub and could
disagree about what is skipped; kept per machine, they ask once between them and a skip applies to
the binary it is about. So it lives in the platform's own state directory rather than in
`.layover/`. A check is never made by reading `GET /update`: the dashboard reads what the Tower last
found, so a `--watch-only` dashboard says the same as the Tower beside it and opening a page costs
no request.

**Why `layover about` exists.** Before anything trusts a binary it has downloaded, it must ask that
binary what it is: its version and platform, to refuse one that is not what the manifest promised,
and the layout of the state directory it reads, because a layout change turns an update into a
migration a person should choose. Asking the binary rather than a second file means the answer
comes from what was verified. So every release from now on carries `layover about --json`, and
`min_upgrade_from` with it — the oldest release one can be reached from in one step, raised only by
a release that cannot be.

**Why provenance is checked by `gh`, not inside the binary.** A checksum fetched from the same place
as the archive proves only that the two agree; a compromised release would carry a matching one.
Every release carries a GitHub build-provenance attestation, signed through Sigstore with the
identity of the workflow that built it, and that is what says who built it. The plan was to verify
it in the binary with the `sigstore` crate, with `gh` as a fallback (the maintainer, 15 October
2026). A spike against the genuine 1.14.0 bundle rejected the crate: it refused the bundle
("signature transparency materials are inconsistent"), left the transparency-log inclusion proof and
the signed entry timestamp as unimplemented TODOs, and brought 332 crates, an 11 MB binary and
aws-lc-rs with it. `gh attestation verify` checked the same bundle in four seconds, and refuses a
wrong ref or a tampered file. So `gh` is the verifier, pinned to the repository, the release
workflow, the tag, the GitHub Actions issuer and the SLSA provenance predicate. What that costs is a
dependency on a tool Layover does not ship: a machine without `gh`, or with it signed out, can
download and checksum a release but not verify it — and an unverified release is never run, never
installed unattended, and installed by hand only on an explicit `--trust-checksum-only`. A failure
that is about reaching GitHub or signing in reads as *could not check*; anything else `gh` refuses
is a refusal, and goes no further.

**Why nothing downloaded runs before it is verified.** Asking a staged binary what it is means
running it, with the user's privileges. Run before its provenance is known, a compromised release
would be executed by the very step meant to vet it — automatically, under `mode = "stage"`, with
nobody watching. So the binary is not even unpacked until `gh` has verified the archive, and runs
with an environment stripped of everything but what a process needs to start, `LAYOVER_UPDATE=off`
so it cannot reach out itself, and twenty seconds to answer. The checksum and the size cap come
first because they cost nothing to check and refuse the commonest failure, a truncated or wrong
download, without asking anyone.

**Why the gate compares warnings rather than counting them.** A release that refuses a factory must
not be installed into it, so errors refuse. Warnings are subtler: a factory may carry one on purpose
— the maintainer's has an `overlap = "allow"` warning it has always had — and refusing on any
warning would hold that factory back for ever. A warning the new release gives and the running one
does not is news, though, and news is for a person: it holds back an unattended install and is shown
before a manual one. The comparison is by the warning's text, so a reworded warning reads as new;
that errs toward asking. Free space is not checked before downloading, though the plan proposed it:
the archive is a few megabytes, standard Rust has no portable way to ask, and a write that fails for
want of space fails the staging cleanly and leaves nothing behind.

**Why an install drains, and cuts off only at a deadline.** Installing a release means restarting
the Tower, and restarting it while runs are alive is what draining was built to avoid. So an install
waits — nothing new starts, what is running finishes — for as long as `[update] max_drain` says,
four hours by default because that is as long as the reference factory lets a run take: with the
deadline at least the longest `timeout`, nothing alive when the drain began is still alive at it.
Waiting for ever would let one long run hold every update back (the maintainer chose a deadline that
cuts runs off, 15 October 2026); a run cut off is recorded `interrupted` and restarted on the new
version by the recovery that already exists for a Tower that went away. The exception is an agent
whose `recovery` is `manual`: that setting says its work must not happen twice — a run cut off
halfway through opening a pull request would open a second — so while one runs at the deadline the
install is postponed, and the Tower carries on, rather than cutting it off. `--now` reaches the
deadline at once and keeps the exception; `--including-manual` drops it, for a person who means it.

**Why Windows hands over in place, and systemd and launchd restart.** A Tower must not run two
versions at once, and must not be left down. Under systemd and launchd the service manager ends
every process in the service when its main one exits, so a successor the old Tower started would be
killed with it; there the old Tower exits 75 — `EX_TEMPFAIL`, which both restart — and the manager
starts the new binary. A spike on the maintainer's machine showed the Windows Task Scheduler does
not restart a task that exits non-zero, and its five-minute watchdog is slow; but a child started
with `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` outlives the task's command (and adding
`CREATE_BREAKAWAY_FROM_JOB` is refused). So on Windows, and anywhere without a service manager, the
old Tower closes its dashboard, starts the new binary at the same address, and waits for it to
answer as the new version before it exits. That has a property the restart path lacks: the old
process supervises the new one, so a release that cannot start at all is rolled back by the Tower it
was meant to replace. `LAYOVER_SUPERVISOR`, which the service files `autostart` writes now set,
selects the restart path; a systemd service written before that is recognised by the `INVOCATION_ID`
systemd gives every service.

**Why the new Tower keeps the old one's token.** A dashboard token is minted when `serve` starts,
and a page holds it in a cookie. A Tower restarted by an update would mint a new one, and every
open page would answer 401 until someone found the new address in a log — on every update. So the
old Tower writes its token into the handover record (readable by its user only), and the new one
adopts it and removes it from the record. The token never leaves the factory's own directory, which
its user already owns.

**Why one Tower serves a factory.** Two Towers over one factory race for its queue, which risk 23
has always described; nothing stopped it. An update makes it sharper: the request to install is a
file, and two Towers would both answer it. So `serve` now takes a lock on `.layover/serving.lock`
for as long as it lives, and a second `serve` on the same factory refuses to start. The lock is how
`layover update apply` tells whether a Tower is there to ask, and the one thing a Tower gives up
during a handover, for the new Tower to take. `layover run` takes no such lock: it is the one-shot
command, and its old freedom to run beside a `serve` is unchanged.

**Why healthy means once round the loop and an answer.** A new version has to show it can do the
job, not only that it started: it opened the state directory, loaded and validated the factory —
both refuse to start otherwise — went once round its loop, which reads the queue and settles what it
finds, and answered its own `/health` as the new version, within ten minutes of the handover. Then
the update is logged and the old binary kept for `keep_previous`. Under a service manager, a new
version that has started twice without that, or run out of time, puts the old binary back itself and
exits 75 for the manager to start it.

**Why a package manager's Layover is left to it, and a layout change to a person.** The plan was to
drain and then run `brew upgrade` or `npm install -g` as the swap. A package manager owns its files
and its records; replacing its binary under it leaves both wrong, and running it from inside a Tower
is a path no test here can exercise. So Layover says the command and installs nothing. A release
that reads a different state-directory layout is refused by `apply` for now too: going back from it
needs a snapshot of what the migration touched, which is not built.

**Why unattended installs stop at minor, wait out a cool-off, and need `gh`.** An install with
nobody watching is the one that has to be safest, so its limits were chosen with the maintainer on
15 October 2026. It may take a patch or a minor release (`allow = "minor"` by default, after a first
answer of `"patch"` was corrected) and never a major one, whose number says something incompatible
changed; nor one that changes the state layout, whose way back is not built. It waits a day for a
patch and two for a minor, counted from when the release was published — the time in which a release
found to be bad can be withdrawn before unattended factories have it — and a minor waits longer
because it changes more. It installs only what `gh` verified, by default: the checksum alone proves
the file matches its manifest, not who built either. And it installs nothing whose own validation of
the factory says something the running version does not, because that is news a person should read
first. Everything it will not take is still offered, and installed by hand with one command.

**Why a window that closes postpones rather than cuts off.** A person who chose a window chose when
work may be interrupted. An install begun in it that is still waiting for runs when it closes has
left that window, so it gives up for this one and work starts again; the next window tries again.
Cutting off at the window's end would interrupt work at a time the person said not to. `max_drain`
still applies inside the window, as it does to an install by hand. With no window, an install
begins only once nothing has run, started, been parked or been queued for `idle_for`; the drain that
follows makes it safe whatever happens in between, so the idle reckoning, sampled once a minute,
needs only to pick a quiet moment, not to prove one.
