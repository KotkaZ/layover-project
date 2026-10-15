//! `GET /update`: what this machine last learned about releases, as the dashboard reads it.

use std::fs;
use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use layover_dashboard::{Dashboard, DashboardState, Guard, router};
use layover_store::{History, Journal};
use layover_update::{Channel, LastCheck, Release, Settings, Source, Version, state};
use tower::ServiceExt as _;

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-dash-update-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("history")).expect("a temporary directory");
        Self(path)
    }

    fn settings(&self) -> Settings {
        Settings {
            switched_off: false,
            // Never asked: reading the status makes no request.
            source: Source::mirror("http://127.0.0.1:9"),
            dir: self.0.join("machine"),
        }
    }

    fn dashboard(&self, factory: &str) -> Dashboard {
        fs::write(self.0.join("layover.toml"), factory).expect("writes");
        Dashboard::new(DashboardState {
            config_path: self.0.join("layover.toml"),
            history: History::open(self.0.join("history")).expect("opens"),
            journal: std::sync::Arc::new(Journal::open(self.0.join("journal")).expect("opens")),
            ground_stop: self.0.join("ground-stop"),
        })
        .updates(self.settings())
    }

    /// A dashboard with no Tower behind it, as `serve --watch-only` serves.
    fn router(&self, factory: &str) -> Router {
        router(self.dashboard(factory), Guard::Open)
    }

    /// A dashboard in the same process as the factory's Tower.
    fn towered(&self, factory: &str) -> Router {
        router(
            self.dashboard(factory).dispatched_by("the test's Tower"),
            Guard::Open,
        )
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const FACTORY: &str =
    "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\n";

async fn update(app: Router) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::builder()
                .uri("/update")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("answered");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON")
}

fn next_minor() -> Version {
    let mut version = Version::running();
    version.minor += 1;
    version.patch = 0;
    version.pre.clear();
    version
}

fn found(temp: &Temp, version: &Version) {
    let at = "2026-10-18T09:00:00Z".parse().expect("valid");
    state::record(
        &temp.settings().dir,
        &LastCheck {
            channel: Channel::Stable,
            attempted_at: at,
            succeeded_at: Some(at),
            error: None,
            failures: 0,
            newest: Some(Release {
                version: version.clone(),
                tag: format!("v{version}"),
                prerelease: false,
                title: Some(format!("{version} — 2026-10-17")),
                notes: "Summary.\n\n**Compatibility.** Additive.\n\n### Added\n".to_owned(),
                published_at: Some("2026-10-17T12:00:00Z".parse().expect("valid")),
                page: format!("https://example.invalid/tag/v{version}"),
            }),
            etag: None,
        },
    )
    .expect("records");
}

#[tokio::test]
async fn a_newer_release_the_machine_found_is_available_with_what_it_changes() {
    let temp = Temp::new("found");
    let next = next_minor();
    found(&temp, &next);

    let said = update(temp.router(FACTORY)).await;

    assert_eq!(said["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(said["mode"], "notify");
    assert_eq!(said["channel"], "stable");
    assert_eq!(said["switched_off"], false);
    assert_eq!(said["latest"], next.to_string());
    assert_eq!(said["checked_at"], "2026-10-18T09:00:00Z");
    let available = &said["available"];
    assert_eq!(available["version"], next.to_string());
    assert_eq!(available["kind"], "minor");
    assert_eq!(available["compatibility"], "**Compatibility.** Additive.");
    assert_eq!(available["published_at"], "2026-10-17T12:00:00Z");
}

#[tokio::test]
async fn nothing_is_available_when_it_is_skipped_or_nothing_was_ever_checked() {
    let temp = Temp::new("skipped");
    let never = update(temp.router(FACTORY)).await;
    assert!(never["available"].is_null(), "{never}");
    assert!(never["checked_at"].is_null(), "{never}");

    let next = next_minor();
    found(&temp, &next);
    state::skip(&temp.settings().dir, &next).expect("skips");
    let skipped = update(temp.router(FACTORY)).await;

    assert!(skipped["available"].is_null(), "{skipped}");
    assert_eq!(skipped["skipped"][0], next.to_string());
    assert_eq!(skipped["latest"], next.to_string(), "still said to exist");
}

#[tokio::test]
async fn the_factorys_own_settings_are_reported() {
    let temp = Temp::new("settings");
    let said = update(temp.router(&format!(
        "{FACTORY}[update]\nmode = \"off\"\nchannel = \"prerelease\"\n"
    )))
    .await;

    assert_eq!(said["mode"], "off");
    assert_eq!(said["channel"], "prerelease");
}

#[tokio::test]
async fn a_staged_release_says_how_far_it_got_and_what_it_made_of_this_factory() {
    let temp = Temp::new("staged");
    let next = next_minor();
    found(&temp, &next);
    let app = temp.router(FACTORY);
    let staged = layover_update::Staged {
        version: next.clone(),
        tag: format!("v{next}"),
        at: "2026-10-18T10:00:00Z".parse().expect("valid"),
        archive: "layover-cli.zip".to_owned(),
        sha256: Some("a".repeat(64)),
        attestation: Some(layover_update::Attestation::Verified {
            signer: "release.yml@refs/tags/v".to_owned(),
        }),
        trusted_checksum_only: false,
        binary: Some(temp.0.join("layover")),
        about: Some(layover_update::About::of_this_binary(1, "x")),
        crosses_layout: false,
        refused: None,
    };
    let dir = layover_update::stage::dir_for(&temp.settings(), &next);
    fs::create_dir_all(&dir).expect("creates");
    fs::write(
        dir.join("staged.json"),
        serde_json::to_string(&staged).expect("serialises"),
    )
    .expect("writes");
    layover_update::gate::record(&layover_update::Gate {
        version: next.clone(),
        factory: temp.0.join("layover.toml"),
        at: "2026-10-18T10:01:00Z".parse().expect("valid"),
        not_run: None,
        errors: Vec::new(),
        new_warnings: vec!["something new".to_owned()],
    })
    .expect("records");

    let said = update(app).await;

    assert_eq!(said["staged"]["version"], next.to_string());
    assert_eq!(said["staged"]["provenance"], "verified");
    assert_eq!(said["staged"]["ready"], true);
    assert_eq!(said["gate"]["passes"], true);
    assert_eq!(said["gate"]["passes_unattended"], false);
    assert_eq!(said["gate"]["new_warnings"][0], "something new");
}

async fn send(
    app: Router,
    method: &str,
    path: &str,
    body: &str,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("answered");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn stage_ready(temp: &Temp, next: &Version, passes: bool) {
    let staged = layover_update::Staged {
        version: next.clone(),
        tag: format!("v{next}"),
        at: "2026-10-18T10:00:00Z".parse().expect("valid"),
        archive: "layover-cli.zip".to_owned(),
        sha256: Some("a".repeat(64)),
        attestation: Some(layover_update::Attestation::Verified {
            signer: "release.yml".to_owned(),
        }),
        trusted_checksum_only: false,
        binary: Some(temp.0.join("layover")),
        about: Some(layover_update::About::of_this_binary(1, "x")),
        crosses_layout: false,
        refused: None,
    };
    let dir = layover_update::stage::dir_for(&temp.settings(), next);
    fs::create_dir_all(&dir).expect("creates");
    fs::write(
        dir.join("staged.json"),
        serde_json::to_string(&staged).expect("serialises"),
    )
    .expect("writes");
    layover_update::gate::record(&layover_update::Gate {
        version: next.clone(),
        factory: temp.0.join("layover.toml"),
        at: "2026-10-18T10:01:00Z".parse().expect("valid"),
        not_run: None,
        errors: if passes {
            Vec::new()
        } else {
            vec!["no".to_owned()]
        },
        new_warnings: Vec::new(),
    })
    .expect("records");
}

#[tokio::test]
async fn a_dashboard_with_no_tower_behind_it_does_not_pretend_to_install() {
    let temp = Temp::new("watching");
    let next = next_minor();
    found(&temp, &next);
    stage_ready(&temp, &next, true);

    let (status, said) = send(temp.router(FACTORY), "POST", "/update/apply", "{}").await;

    assert_eq!(status, StatusCode::CONFLICT, "{said}");
    assert!(said.to_string().contains("no Tower runs here"), "{said}");
    assert!(
        !temp
            .0
            .join(".layover")
            .join("update")
            .join("request.json")
            .exists()
    );
}

#[tokio::test]
async fn the_tower_is_asked_to_install_a_staged_release_and_the_ask_can_be_withdrawn() {
    let temp = Temp::new("ask");
    let next = next_minor();
    found(&temp, &next);

    let (refused, said) = send(temp.towered(FACTORY), "POST", "/update/apply", "{}").await;
    assert_eq!(refused, StatusCode::CONFLICT, "nothing staged yet: {said}");

    stage_ready(&temp, &next, true);
    let (asked, said) = send(
        temp.towered(FACTORY),
        "POST",
        "/update/apply",
        r#"{"now":true}"#,
    )
    .await;
    assert_eq!(asked, StatusCode::ACCEPTED, "{said}");
    assert_eq!(said["request"]["version"], next.to_string());
    assert_eq!(said["request"]["by"], "the dashboard");
    assert!(said["request"]["outcome"].is_null());
    let request = layover_update::handover::request(&temp.0.join(".layover")).expect("written");
    assert!(request.now);

    let (again, _) = send(temp.towered(FACTORY), "POST", "/update/apply", "{}").await;
    assert_eq!(again, StatusCode::CONFLICT, "one at a time");

    let (cancelled, said) = send(temp.towered(FACTORY), "DELETE", "/update/apply", "").await;
    assert_eq!(cancelled, StatusCode::OK);
    assert_eq!(said["request"]["outcome"], "cancelled");
}

#[tokio::test]
async fn a_release_that_refuses_this_factory_is_not_installed() {
    let temp = Temp::new("refuses");
    let next = next_minor();
    found(&temp, &next);
    stage_ready(&temp, &next, false);

    let (status, said) = send(temp.towered(FACTORY), "POST", "/update/apply", "{}").await;

    assert_eq!(status, StatusCode::CONFLICT, "{said}");
    assert!(said.to_string().contains("refuses this factory"), "{said}");
}

#[tokio::test]
async fn a_version_is_skipped_from_the_dashboard_and_a_rollback_needs_one_kept() {
    let temp = Temp::new("skip-roll");
    let next = next_minor();
    found(&temp, &next);

    let (bad, _) = send(
        temp.router(FACTORY),
        "POST",
        "/update/skip",
        r#"{"version":"soon"}"#,
    )
    .await;
    assert_eq!(bad, StatusCode::BAD_REQUEST);
    let (skipped, said) = send(
        temp.router(FACTORY),
        "POST",
        "/update/skip",
        &format!(r#"{{"version":"{next}"}}"#),
    )
    .await;
    assert_eq!(skipped, StatusCode::OK);
    assert!(said["available"].is_null());
    assert_eq!(said["skipped"][0], next.to_string());

    let (nothing, said) = send(temp.towered(FACTORY), "POST", "/update/rollback", "").await;
    assert_eq!(nothing, StatusCode::CONFLICT, "{said}");
}
