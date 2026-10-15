//! Discovering a release: which one is newer, on which channel, what is skipped, and that a
//! machine switched off asks nothing at all.

use jiff::Timestamp;
use layover_update::{
    Channel, CheckError, Kind, Mode, Settings, Source, UpdateConfig, Version, check, state, status,
};

mod fake;
use fake::{Answer, Fake, Temp, manifest};

const NOTES: &str = "Better.\n\n**Compatibility.** Additive for `layover.toml`.\n\n### Added\n";

fn settings(fake: &Fake, temp: &Temp) -> Settings {
    Settings {
        switched_off: false,
        source: Source::mirror(&fake.releases()),
        dir: temp.0.clone(),
    }
}

fn v(text: &str) -> Version {
    text.parse().expect("valid")
}

fn now() -> Timestamp {
    "2026-10-18T09:00:00Z".parse().expect("valid")
}

fn config(channel: Channel) -> UpdateConfig {
    UpdateConfig {
        channel,
        ..UpdateConfig::default()
    }
}

#[test]
fn a_newer_stable_release_is_found_and_said_to_be_available() {
    let fake = Fake::start();
    let temp = Temp::new("found");
    fake.publish(
        "v1.15.0",
        &manifest("1.15.0", false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    let settings = settings(&fake, &temp);

    let release = check(&settings, Channel::Stable, now()).expect("found");

    assert_eq!(release.version, v("1.15.0"));
    assert_eq!(release.tag, "v1.15.0");
    assert_eq!(
        release.published_at,
        Some("2026-10-17T12:00:00Z".parse().expect("valid")),
        "from the upload time"
    );
    assert_eq!(release.page, format!("{}/tag/v1.15.0", fake.releases()));
    assert!(release.notes.contains("**Compatibility.**"));

    let status = status(&settings, &config(Channel::Stable), &v("1.14.0"));
    let available = status.available.expect("available");
    assert_eq!(available.release.version, v("1.15.0"));
    assert_eq!(available.kind, Kind::Minor);
    assert_eq!(status.mode, Mode::Notify);
    let last = status.last.expect("recorded");
    assert_eq!(last.succeeded_at, Some(now()));
    assert_eq!(last.error, None);

    let up_to_date = status_for(&settings, "1.15.0");
    assert!(up_to_date.is_none(), "nothing newer than what runs");
}

fn status_for(settings: &Settings, running: &str) -> Option<layover_update::Available> {
    status(settings, &config(Channel::Stable), &v(running)).available
}

#[test]
fn an_unchanged_manifest_is_not_downloaded_again() {
    let fake = Fake::start();
    let temp = Temp::new("etag");
    fake.publish(
        "v1.15.0",
        &manifest("1.15.0", false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    let settings = settings(&fake, &temp);

    check(&settings, Channel::Stable, now()).expect("found");
    let again = check(&settings, Channel::Stable, now()).expect("still found");

    assert_eq!(again.version, v("1.15.0"));
    let asked: Vec<_> = fake
        .seen()
        .into_iter()
        .filter(|seen| seen.path.ends_with("v1.15.0/dist-manifest.json"))
        .collect();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].headers.get("if-none-match"), None);
    assert_eq!(
        asked[1].headers.get("if-none-match").map(String::as_str),
        Some("\"v1.15.0\"")
    );
}

#[test]
fn the_prerelease_channel_takes_the_newest_listed_release_and_stable_does_not() {
    let fake = Fake::start();
    let temp = Temp::new("channels");
    fake.publish(
        "v1.14.1",
        &manifest("1.14.1", false, NOTES),
        "Fri, 16 Oct 2026 12:00:00 GMT",
        true,
    );
    fake.publish(
        "v1.15.0-rc.1",
        &manifest("1.15.0-rc.1", true, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        false,
    );
    fake.answer(
        "/releases/releases.json",
        Answer::ok(
            serde_json::json!([
                { "tag_name": "v1.16.0", "draft": true, "prerelease": false },
                { "tag_name": "v1.15.0-rc.1", "draft": false, "prerelease": true,
                  "published_at": "2026-10-17T12:05:00Z" },
                { "tag_name": "v1.14.1", "draft": false, "prerelease": false },
                { "tag_name": "not-a-version", "draft": false, "prerelease": false }
            ])
            .to_string(),
        ),
    );
    let settings = settings(&fake, &temp);

    let candidate = check(&settings, Channel::Prerelease, now()).expect("found");
    assert_eq!(candidate.version, v("1.15.0-rc.1"), "not the draft");
    assert!(candidate.prerelease);
    assert_eq!(
        candidate.published_at,
        Some("2026-10-17T12:05:00Z".parse().expect("valid")),
        "the API's date"
    );
    let stable = check(&settings, Channel::Stable, now()).expect("found");
    assert_eq!(stable.version, v("1.14.1"));

    let on = |channel| status(&settings, &config(channel), &v("1.14.0")).available;
    assert_eq!(
        on(Channel::Prerelease).map(|found| found.release.version),
        Some(v("1.15.0-rc.1"))
    );
    assert_eq!(
        on(Channel::Stable).map(|found| found.release.version),
        Some(v("1.14.1")),
        "each channel keeps its own check"
    );
}

#[test]
fn a_skipped_version_is_not_offered_and_a_newer_one_is() {
    let fake = Fake::start();
    let temp = Temp::new("skip");
    fake.publish(
        "v1.15.0",
        &manifest("1.15.0", false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    let settings = settings(&fake, &temp);
    check(&settings, Channel::Stable, now()).expect("found");

    state::skip(&settings.dir, &v("1.15.0")).expect("skips");
    let skipped = status(&settings, &config(Channel::Stable), &v("1.14.0"));
    assert!(skipped.available.is_none());
    assert_eq!(skipped.skipped, vec![v("1.15.0")]);

    fake.publish(
        "v1.15.1",
        &manifest("1.15.1", false, NOTES),
        "Sun, 18 Oct 2026 08:00:00 GMT",
        true,
    );
    check(&settings, Channel::Stable, now()).expect("found");
    assert_eq!(
        status_for(&settings, "1.14.0").map(|found| found.release.version),
        Some(v("1.15.1"))
    );
}

#[test]
fn a_machine_switched_off_makes_no_request_at_all() {
    let fake = Fake::start();
    let temp = Temp::new("off");
    fake.publish(
        "v1.15.0",
        &manifest("1.15.0", false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    let settings = Settings {
        switched_off: true,
        ..settings(&fake, &temp)
    };

    assert_eq!(
        check(&settings, Channel::Stable, now()),
        Err(CheckError::SwitchedOff)
    );
    assert!(fake.seen().is_empty(), "{:?}", fake.seen());
    assert!(state::last_check(&settings.dir, Channel::Stable).is_none());
    assert!(status(&settings, &config(Channel::Stable), &v("1.14.0")).switched_off);
}

#[test]
fn a_failed_check_is_recorded_and_keeps_what_the_last_one_found() {
    let fake = Fake::start();
    let temp = Temp::new("failed");
    fake.publish(
        "v1.15.0",
        &manifest("1.15.0", false, NOTES),
        "Sat, 17 Oct 2026 12:00:00 GMT",
        true,
    );
    let settings = settings(&fake, &temp);
    check(&settings, Channel::Stable, now()).expect("found");

    fake.answer(
        "/releases/latest/download/dist-manifest.json",
        Answer::status(502),
    );
    let later: Timestamp = "2026-10-19T09:00:00Z".parse().expect("valid");
    let failed = check(&settings, Channel::Stable, later).expect_err("fails");
    assert!(failed.to_string().contains("502"), "{failed}");

    let last = state::last_check(&settings.dir, Channel::Stable).expect("recorded");
    assert_eq!(last.attempted_at, later);
    assert_eq!(last.succeeded_at, Some(now()), "when it last worked");
    assert_eq!(last.failures, 1);
    assert!(last.error.is_some_and(|error| error.contains("502")));
    assert_eq!(
        status_for(&settings, "1.14.0").map(|found| found.release.version),
        Some(v("1.15.0")),
        "what was found is still said"
    );
}

#[test]
fn something_that_is_not_a_manifest_is_refused() {
    let fake = Fake::start();
    let temp = Temp::new("garbage");
    fake.answer(
        "/releases/latest/download/dist-manifest.json",
        Answer::ok("<html>a captive portal</html>"),
    );

    let failed = check(&settings(&fake, &temp), Channel::Stable, now()).expect_err("refused");
    assert!(
        failed
            .to_string()
            .contains("did not return what was expected"),
        "{failed}"
    );
}
