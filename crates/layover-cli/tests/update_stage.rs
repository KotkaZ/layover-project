//! Staging a release with a real binary in it, and the gate reading what a real `layover validate`
//! prints.

use std::path::{Path, PathBuf};
use std::process::Command;

use layover_update::provenance::{Attestation, Verifier};
use layover_update::stage::Running;
use layover_update::{About, Release, Settings, Source, Staged, Version, gate, stage};

#[path = "../../layover-update/tests/fake/mod.rs"]
mod fake;
use fake::{Fake, Temp, archive, sha256};

struct Verified;

impl Verifier for Verified {
    fn verify(&self, _: &Path, tag: &str) -> Attestation {
        Attestation::Verified {
            signer: format!("release.yml@refs/tags/{tag}"),
        }
    }
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    }
}

/// What a release of this version carries: on Windows the real binary; elsewhere, where a debug
/// build with its symbols outgrows the download cap, a script that answers `about` as it would.
fn release_binary() -> Vec<u8> {
    if cfg!(windows) {
        std::fs::read(env!("CARGO_BIN_EXE_layover")).expect("reads the binary")
    } else {
        let about = serde_json::to_string(&About::of_this_binary(
            layover_store::layout::CURRENT,
            env!("CARGO_PKG_VERSION"),
        ))
        .expect("serialises");
        format!("#!/bin/sh\nif [ \"$1\" = about ]; then echo '{about}'; exit 0; fi\nexit 3\n")
            .into_bytes()
    }
}

fn publish(fake: &Fake, version: &str) {
    let bytes = archive(&[(binary_name(), &release_binary()), ("README.md", b"readme")]);
    let digest = sha256(&bytes);
    fake.publish_archive(version, &bytes, &digest, &digest);
}

fn release(fake: &Fake, version: &str) -> Release {
    Release {
        version: version.parse().expect("valid"),
        tag: format!("v{version}"),
        prerelease: false,
        title: None,
        notes: String::new(),
        published_at: None,
        page: format!("{}/tag/v{version}", fake.releases()),
    }
}

#[test]
fn a_release_is_downloaded_verified_unpacked_and_asked_what_it_is() {
    let fake = Fake::start();
    let temp = Temp::new("cli-stage");
    let version = env!("CARGO_PKG_VERSION");
    publish(&fake, version);
    let settings = Settings {
        switched_off: false,
        source: Source::mirror(&fake.releases()),
        dir: temp.0.clone(),
    };
    let running = Running {
        version: "1.0.0".parse().expect("valid"),
        layout: layover_store::layout::CURRENT,
    };

    let staged = stage(
        &settings,
        &release(&fake, version),
        &running,
        &Verified,
        false,
    );

    assert_eq!(staged.refused, None, "{staged:?}");
    assert!(staged.is_ready(), "{staged:?}");
    let about = staged.about.as_ref().expect("asked");
    assert_eq!(about.version, version);
    assert_eq!(about.target, layover_update::TARGET);
    assert!(!staged.crosses_layout);
    let binary = staged.binary.as_ref().expect("unpacked");
    assert!(binary.starts_with(&temp.0), "{}", binary.display());
}

/// A staged release whose binary is this test's own `layover`.
fn this_binary() -> Staged {
    Staged {
        version: env!("CARGO_PKG_VERSION").parse().expect("valid"),
        tag: format!("v{}", env!("CARGO_PKG_VERSION")),
        at: jiff::Timestamp::now(),
        archive: "archive".to_owned(),
        sha256: Some("0".repeat(64)),
        attestation: Some(Attestation::Verified {
            signer: "release.yml".to_owned(),
        }),
        trusted_checksum_only: false,
        binary: Some(PathBuf::from(env!("CARGO_BIN_EXE_layover"))),
        about: Some(About::of_this_binary(
            layover_store::layout::CURRENT,
            env!("CARGO_PKG_VERSION"),
        )),
        crosses_layout: false,
        refused: None,
    }
}

fn factory(dir: &Path, text: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("creates");
    let path = dir.join("layover.toml");
    std::fs::write(&path, text).expect("writes");
    path
}

const CLEAN: &str = "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\n\
                     entry = true\ndescription = \"does a\"\n";

#[test]
fn the_gate_measures_a_factory_by_what_the_staged_binary_says() {
    let temp = Temp::new("cli-gate");
    let staged = this_binary();

    let clean = factory(&temp.0.join("clean"), CLEAN);
    let passed = gate(&staged, &clean, &[]);
    assert!(passed.passes_unattended(), "{passed:?}");

    // Without a description: a warning, which the running version is told it never gave.
    let warned = factory(
        &temp.0.join("warned"),
        "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n",
    );
    let new = gate(&staged, &warned, &[]);
    assert!(new.passes() && !new.passes_unattended(), "{new:?}");
    let warning = new.new_warnings.first().expect("one").clone();
    assert!(warning.contains("description"), "{warning}");
    // And one the factory always had holds nothing back.
    assert!(gate(&staged, &warned, &[warning]).passes_unattended());

    let broken = factory(
        &temp.0.join("broken"),
        &format!("{CLEAN}[[routes]]\nfrom = \"a\"\nto = \"ghost\"\n"),
    );
    let refused = gate(&staged, &broken, &[]);
    assert!(!refused.passes(), "{refused:?}");
    assert!(
        refused.errors.iter().any(|error| error.contains("ghost")),
        "{refused:?}"
    );
}

/// `layover` with its update state in `home`, releases from `fake`, and no `gh` to be found.
fn layover(home: &Path, fake: &Fake, args: &[&str]) -> (bool, String) {
    let empty = home.join("no-tools");
    std::fs::create_dir_all(&empty).expect("creates");
    let output = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(args)
        .current_dir(home)
        .env("LAYOVER_UPDATE_SOURCE", fake.releases())
        .env_remove("LAYOVER_UPDATE")
        .env("PATH", &empty)
        .env("LOCALAPPDATA", home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .output()
        .expect("runs");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

fn next_minor() -> String {
    let mut version = Version::running();
    version.minor += 1;
    version.patch = 0;
    version.pre.clear();
    version.to_string()
}

fn announced(fake: &Fake, version: &str) {
    fake.publish(
        &format!("v{version}"),
        &fake::manifest(version, false, "Notes."),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    publish(fake, version);
}

#[test]
fn without_gh_a_release_is_staged_but_not_run() {
    let fake = Fake::start();
    let home = Temp::new("cli-stage-nogh");
    factory(&home.0, CLEAN);
    let next = next_minor();
    announced(&fake, &next);

    let (ok, said) = layover(&home.0, &fake, &["update", "stage"]);

    assert!(ok, "{said}");
    assert!(said.contains("matches the release"), "{said}");
    assert!(
        said.contains("Provenance not verified: the GitHub CLI, gh, is not installed"),
        "{said}"
    );
    assert!(said.contains("So it was not run."), "{said}");
    assert!(
        said.contains("Not yet checked against"),
        "the gate is not run either: {said}"
    );
}

#[test]
fn a_binary_that_is_not_the_release_it_came_in_is_refused() {
    let fake = Fake::start();
    let home = Temp::new("cli-stage-liar");
    factory(&home.0, CLEAN);
    // The archive's binary says it is this version; the release says it is the next.
    let next = next_minor();
    announced(&fake, &next);

    let (ok, said) = layover(
        &home.0,
        &fake,
        &["update", "stage", "--trust-checksum-only"],
    );

    assert!(!ok, "{said}");
    assert!(
        said.contains(&format!(
            "it says it is {}, but the release is {next}",
            env!("CARGO_PKG_VERSION")
        )),
        "{said}"
    );
}
