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

    fn router(&self, factory: &str) -> Router {
        fs::write(self.0.join("layover.toml"), factory).expect("writes");
        router(
            Dashboard::new(DashboardState {
                config_path: self.0.join("layover.toml"),
                history: History::open(self.0.join("history")).expect("opens"),
                journal: std::sync::Arc::new(Journal::open(self.0.join("journal")).expect("opens")),
                ground_stop: self.0.join("ground-stop"),
            })
            .updates(self.settings()),
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
