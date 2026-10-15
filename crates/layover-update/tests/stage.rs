//! Staging a release: every way it is refused, and that nothing downloaded runs before its
//! provenance is verified.

use layover_update::provenance::{Attestation, Verifier};
use layover_update::stage::{Running, dir_for};
use layover_update::{Release, Settings, Source, Version, gate, stage};

mod fake;
use fake::{Fake, Temp, archive, sha256};

/// A verifier that says what it is told to, and remembers it was asked.
struct Says(Attestation, std::cell::Cell<usize>);

impl Says {
    fn verified() -> Self {
        Self(
            Attestation::Verified {
                signer: "release.yml@refs/tags/v1.15.0".to_owned(),
            },
            std::cell::Cell::new(0),
        )
    }

    fn unavailable() -> Self {
        Self(
            Attestation::Unavailable {
                reason: "gh is not installed".to_owned(),
            },
            std::cell::Cell::new(0),
        )
    }

    fn refused() -> Self {
        Self(
            Attestation::Refused {
                reason: "no attestations found".to_owned(),
            },
            std::cell::Cell::new(0),
        )
    }
}

impl Verifier for Says {
    fn verify(&self, archive: &std::path::Path, _: &str) -> Attestation {
        assert!(archive.exists(), "verified once downloaded");
        self.1.set(self.1.get() + 1);
        self.0.clone()
    }
}

const VERSION: &str = "1.15.0";

fn binary() -> &'static str {
    if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    }
}

fn settings(fake: &Fake, temp: &Temp) -> Settings {
    Settings {
        switched_off: false,
        source: Source::mirror(&fake.releases()),
        dir: temp.0.clone(),
    }
}

fn release(fake: &Fake) -> Release {
    Release {
        version: VERSION.parse().expect("valid"),
        tag: format!("v{VERSION}"),
        prerelease: false,
        title: None,
        notes: String::new(),
        published_at: None,
        page: format!("{}/tag/v{VERSION}", fake.releases()),
    }
}

fn running() -> Running {
    Running {
        version: "1.14.0".parse().expect("valid"),
        layout: 1,
    }
}

/// Publishes `bytes` as the release's archive, listed and side-carred honestly.
fn honest(fake: &Fake, bytes: &[u8]) {
    let digest = sha256(bytes);
    fake.publish_archive(VERSION, bytes, &digest, &digest);
}

fn refused(staged: &layover_update::Staged) -> &str {
    staged.refused.as_deref().unwrap_or("")
}

#[test]
fn an_archive_that_is_not_the_one_listed_is_refused_and_deleted() {
    let fake = Fake::start();
    let temp = Temp::new("stage-checksum");
    let bytes = archive(&[(binary(), b"not really a binary")]);
    let wrong = sha256(b"something else");
    fake.publish_archive(VERSION, &bytes, &wrong, &wrong);
    let verifier = Says::verified();

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &verifier,
        false,
    );

    assert!(
        refused(&staged).contains(&format!("but the release lists {wrong}")),
        "{staged:?}"
    );
    assert_eq!(verifier.1.get(), 0, "never verified, never run");
    assert!(!staged.is_ready());
    let left: Vec<_> = std::fs::read_dir(dir_for(&settings(&fake, &temp), &staged.version))
        .expect("the record is kept")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["staged.json"], "the archive is gone");
}

#[test]
fn a_checksum_file_that_disagrees_with_the_manifest_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("stage-sidecar");
    let bytes = archive(&[(binary(), b"x")]);
    fake.publish_archive(VERSION, &bytes, &sha256(&bytes), &sha256(b"other"));

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::verified(),
        false,
    );

    assert!(refused(&staged).contains(".sha256 says"), "{staged:?}");
}

#[test]
fn an_archive_whose_provenance_is_refused_is_never_unpacked_or_run() {
    let fake = Fake::start();
    let temp = Temp::new("stage-refused");
    honest(&fake, &archive(&[(binary(), b"x")]));

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::refused(),
        true,
    );

    assert!(
        refused(&staged).contains("provenance did not verify: no attestations found"),
        "even trusting the checksum does not override a refusal: {staged:?}"
    );
    assert!(staged.binary.is_none() && staged.about.is_none());
    assert!(
        !dir_for(&settings(&fake, &temp), &staged.version)
            .join(binary())
            .exists()
    );
}

#[test]
fn an_archive_whose_provenance_cannot_be_checked_is_kept_but_not_run() {
    let fake = Fake::start();
    let temp = Temp::new("stage-unavailable");
    honest(&fake, &archive(&[(binary(), b"x")]));
    let settings = settings(&fake, &temp);

    let staged = stage(
        &settings,
        &release(&fake),
        &running(),
        &Says::unavailable(),
        false,
    );

    assert_eq!(staged.refused, None, "{staged:?}");
    assert!(!staged.is_ready(), "not ready to install unattended");
    assert!(staged.sha256.is_some());
    assert!(staged.binary.is_none() && staged.about.is_none(), "not run");
    let dir = dir_for(&settings, &staged.version);
    assert!(
        dir.join(&staged.archive).exists(),
        "kept for a person to trust"
    );
    assert!(!dir.join(binary()).exists());
    assert_eq!(
        layover_update::staged(&settings, &staged.version),
        Some(staged.clone())
    );

    let gate = gate(&staged, &temp.0.join("layover.toml"), &[]);
    assert!(
        gate.not_run
            .as_deref()
            .is_some_and(|why| why.contains("provenance")),
        "{gate:?}"
    );
}

#[test]
fn an_archive_with_an_entry_that_would_land_outside_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("stage-unsafe");
    honest(&fake, &archive(&[(binary(), b"x"), ("../outside", b"y")]));

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::verified(),
        false,
    );

    assert!(refused(&staged).contains("`../outside`"), "{staged:?}");
    assert!(!temp.0.join("staging").join("outside").exists());
}

#[test]
fn an_archive_without_the_binary_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("stage-empty");
    honest(&fake, &archive(&[("README.md", b"hello")]));

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::verified(),
        false,
    );

    assert!(
        refused(&staged).contains(&format!("holds no `{}`", binary())),
        "{staged:?}"
    );
}

#[test]
fn a_binary_that_cannot_say_what_it_is_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("stage-mute");
    honest(&fake, &archive(&[(binary(), b"#not a program\n")]));

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::verified(),
        false,
    );

    assert!(
        refused(&staged).contains("did not say what it is"),
        "{staged:?}"
    );
    assert!(staged.binary.is_none(), "removed once refused");
}

#[test]
fn a_release_with_no_archive_for_this_platform_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("stage-platform");
    fake.answer(
        &format!("/releases/download/v{VERSION}/dist-manifest.json"),
        fake::Answer::ok(fake::manifest(VERSION, false, "notes")),
    );

    let staged = stage(
        &settings(&fake, &temp),
        &release(&fake),
        &running(),
        &Says::verified(),
        false,
    );

    assert!(
        refused(&staged).contains(&format!("has no archive for {}", layover_update::TARGET)),
        "{staged:?}"
    );
}

#[test]
fn staging_a_release_forgets_the_one_staged_before() {
    let fake = Fake::start();
    let temp = Temp::new("stage-one");
    let settings = settings(&fake, &temp);
    let older: Version = "1.14.5".parse().expect("valid");
    std::fs::create_dir_all(dir_for(&settings, &older)).expect("creates");
    honest(&fake, &archive(&[(binary(), b"x")]));

    stage(
        &settings,
        &release(&fake),
        &running(),
        &Says::unavailable(),
        false,
    );

    assert!(!dir_for(&settings, &older).exists());
    layover_update::stage::forget_installed(&settings, &VERSION.parse().expect("valid"));
    assert!(!dir_for(&settings, &VERSION.parse().expect("valid")).exists());
}
