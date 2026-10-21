# Releasing Layover

For maintainers. Releases are built by [`dist`](https://axodotdev.github.io/cargo-dist/), configured
in `dist-workspace.toml`; pushing a tag is what publishes one. Everything before the tag is making
sure the tag is right, because installed Layovers read every release.

## The version lives in four places

Missing any one fails `cargo xtask verify` a step at a time rather than all at once:

1. `version` in the workspace `Cargo.toml`.
2. The inter-crate dependency versions in every `crates/*/Cargo.toml` — `layover-core = { path =
   "…", version = "1.7.0" }`. Cargo refuses to resolve the workspace until these agree.
3. `version:` in `api/openapi.yaml`.
4. `cargo xtask generate-api`, because (3) changes the generated file.

## Steps

1. Bump the version in the four places above.
2. Give the release its `CHANGELOG.md` section, moving what `[Unreleased]` holds under the new
   heading with the date. It needs a **`**Compatibility.**` paragraph** — `verify` refuses a
   section without one — because the updater shows it, before a person installs the release and on
   the dashboard, as what it changes for an existing factory. It says whether the state
   directory's layout changed: if it did, no factory installs it unattended, and the paragraph
   should say what the person installing it is deciding.
3. `cargo xtask verify`, and `dist plan` to see what the tag would build without releasing anything.
4. Commit (`chore: release vX.Y.Z`), push `main`, and wait for CI to pass on that commit.
5. Tag it and push the tag:

   ```sh
   git tag vX.Y.Z      # must match the workspace version in Cargo.toml
   git push origin vX.Y.Z
   ```

   The Release workflow builds all five targets, the installers and checksums, attests each
   archive's provenance, publishes the GitHub Release and updates the Homebrew tap; the SBOM
   workflow attaches one per archive. When the Release workflow finishes, the Pages workflow
   rebuilds the documentation site, whose root follows the latest release.
6. Check what was published: download one archive, compare it with `sha256.sum`, run
   `gh attestation verify <archive> --repo KotkaZ/layover-project`, and run its `layover
   --version`.
7. `cargo publish --workspace`, so `cargo install layover-cli` gets the release too.
8. From a machine on the previous release, `layover update check` must see the new one within
   minutes.

## Things every release must keep

- **`layover about --json` stays**, because the updater asks a downloaded binary what it is before
  trusting it. `MIN_UPGRADE_FROM` in `layover-update` is raised only by a release that cannot be
  reached from an older one directly.
- **A `dist` upgrade refreshes the pinned manifest** in `crates/layover-update/tests/fixtures/`
  from `dist manifest --artifacts=all --output-format=json --no-local-paths`; a test fails until it
  is, so a manifest the updater would misread is caught before a release carries it.
- **`.github/workflows/release.yml` is generated.** `dist host` checks it against what `dist` would
  produce and refuses a release that disagrees. Change `dist-workspace.toml` and run
  `dist init --yes`; never edit the workflow by hand. Hand-editing it has already cost one failed
  release.

## Release candidates, and withdrawing a release

Try the updater against a real release before stable factories do: tag a pre-release,
`vX.Y.Z-rc.1`, which `dist` marks as one (and which skips the Homebrew tap), and update a throwaway
install of the previous release with `[update] channel = "prerelease"` in a throwaway factory. Fold
the candidates' changelog sections into the stable one when it ships.

A bad release is withdrawn by marking it a pre-release on GitHub, which moves `releases/latest`
back to the one before — unattended installs wait a day or two before taking a release, for exactly
this — and a fixed patch follows.
