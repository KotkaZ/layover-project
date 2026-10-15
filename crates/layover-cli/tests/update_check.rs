//! `layover update` and `layover about`, against a release server on loopback.

use std::path::Path;
use std::process::{Command, Output};

#[path = "../../layover-update/tests/fake/mod.rs"]
mod fake;
use fake::{Fake, Temp, manifest};

const NOTES: &str = "Faster.\n\n**Compatibility.** Additive for `layover.toml`.\n- One new key.\n\n\
                     ### Added\n\n- Things.\n";

/// The next minor release after this one.
fn next_minor() -> String {
    let running = env!("CARGO_PKG_VERSION");
    let mut parts = running.split('.');
    let major = parts.next().expect("major");
    let minor: u64 = parts.next().expect("minor").parse().expect("a number");
    format!("{major}.{}.0", minor + 1)
}

/// `layover` with this machine's update state kept in `home`, asking `fake` for releases.
fn layover(home: &Path, fake: &Fake, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(args)
        .current_dir(home)
        .env("LAYOVER_UPDATE_SOURCE", fake.releases())
        .env_remove("LAYOVER_UPDATE")
        .env("LOCALAPPDATA", home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .output()
        .expect("runs")
}

fn said(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn published(fake: &Fake, version: &str) {
    fake.publish(
        &format!("v{version}"),
        &manifest(version, false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
}

#[test]
fn update_check_says_what_is_available_and_what_it_changes() {
    let fake = Fake::start();
    let home = Temp::new("cli-check");
    let next = next_minor();
    published(&fake, &next);

    let output = layover(&home.0, &fake, &["update", "check"]);
    let text = said(&output);

    assert!(output.status.success(), "{text}");
    assert!(
        text.contains(&format!(
            "{next} is available — released 2026-10-17, minor."
        )),
        "{text}"
    );
    assert!(
        text.contains("  **Compatibility.** Additive for `layover.toml`.\n  - One new key."),
        "{text}"
    );
    assert!(
        text.contains(&format!("{}/tag/v{next}", fake.releases())),
        "{text}"
    );
    assert!(
        !text.contains("### Added"),
        "only what an existing factory notices: {text}"
    );

    // `doctor` says the same, from what the check recorded, without asking again.
    std::fs::write(
        home.0.join("layover.toml"),
        "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n",
    )
    .expect("writes");
    let asked = fake.seen().len();
    let doctor = layover(&home.0, &fake, &["doctor"]);
    let first = said(&doctor);
    let first = first.lines().next().unwrap_or_default();
    assert!(
        first.contains(&format!("{next} is available (minor)")),
        "{first}"
    );
    assert!(first.contains("Checked 0 hour(s) ago."), "{first}");
    assert_eq!(fake.seen().len(), asked, "doctor makes no request");
}

#[test]
fn a_skipped_release_is_not_offered_until_the_skip_is_cleared() {
    let fake = Fake::start();
    let home = Temp::new("cli-skip");
    let next = next_minor();
    published(&fake, &next);

    let skipped = layover(&home.0, &fake, &["update", "skip", &next]);
    assert!(skipped.status.success(), "{}", said(&skipped));

    let text = said(&layover(&home.0, &fake, &["update", "check"]));
    assert!(
        text.contains(&format!(
            "{next} is the newest release, and you skipped it."
        )),
        "{text}"
    );

    let cleared = layover(&home.0, &fake, &["update", "skip", "--clear"]);
    assert!(cleared.status.success(), "{}", said(&cleared));
    let text = said(&layover(&home.0, &fake, &["update", "check"]));
    assert!(text.contains(&format!("{next} is available")), "{text}");

    let refused = layover(&home.0, &fake, &["update", "skip", "soon"]);
    assert!(!refused.status.success());
}

#[test]
fn a_machine_switched_off_asks_nothing() {
    let fake = Fake::start();
    let home = Temp::new("cli-off");
    published(&fake, &next_minor());

    let output = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(["update", "check"])
        .current_dir(&home.0)
        .env("LAYOVER_UPDATE_SOURCE", fake.releases())
        .env("LAYOVER_UPDATE", "off")
        .env("LOCALAPPDATA", &home.0)
        .env("HOME", &home.0)
        .env("XDG_STATE_HOME", &home.0)
        .output()
        .expect("runs");

    assert!(!output.status.success());
    assert!(
        said(&output).contains("LAYOVER_UPDATE=off"),
        "{}",
        said(&output)
    );
    assert!(fake.seen().is_empty());
}

#[test]
fn about_json_says_what_an_updater_checks_before_trusting_a_binary() {
    let output = Command::new(env!("CARGO_BIN_EXE_layover"))
        .args(["about", "--json"])
        .output()
        .expect("runs");
    let about: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");

    assert_eq!(about["version"], env!("CARGO_PKG_VERSION"));
    for key in ["target", "api", "min_upgrade_from"] {
        assert!(about[key].is_string(), "{key}: {about}");
    }
    assert!(about["layout"].is_u64(), "{about}");
}
