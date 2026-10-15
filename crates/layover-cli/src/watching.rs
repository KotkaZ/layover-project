//! The Tower's side of updates, on a thread of its own: looking for a release on the factory's
//! schedule, getting it ready under `mode = "stage"` or `"auto"`, and under `"auto"` asking the
//! Tower to install it — in the `window`, or once the factory has been idle for `idle_for` —
//! when the policy allows it to go unattended.
//!
//! Asking is all it does: the install is the same request `layover update apply` writes, drained
//! for, swapped and handed over by the same code. A window that closes while the Tower is still
//! waiting for runs postpones the install to the next one.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use layover_update::handover::{self, Request};
use layover_update::{Available, Mode, Settings, UpdateConfig, Verdict, Version, unattended};

use crate::tower::TowerControl;

/// How often the watcher wakes.
const WAKE: Duration = Duration::from_mins(1);

/// How long an unattended install that did not happen waits before it is tried again, with no
/// window to wait for.
const AGAIN_AFTER: Duration = Duration::from_hours(1);

/// What the watcher needs.
pub struct Watch {
    /// The factory's `[update]`.
    pub config: UpdateConfig,
    /// The factory's definition.
    pub factory: PathBuf,
    /// Its `.layover`.
    pub state: PathBuf,
    /// The Tower, to tell when the factory is idle.
    pub control: TowerControl,
}

/// Starts watching, unless the factory or the machine has checks off.
pub fn watch(watch: Watch, announce: impl Fn(String) + Send + 'static) {
    let settings = Settings::from_env();
    if watch.config.mode == Mode::Off || settings.switched_off {
        return;
    }
    // A release staged before this one was installed is done with.
    layover_update::stage::forget_installed(&settings, &Version::running());
    let spawned = std::thread::Builder::new()
        .name("layover-update".to_owned())
        .spawn(move || Watcher::new(watch, settings).run(&announce));
    if let Err(error) = spawned {
        eprintln!("Could not start looking for updates: {error}");
    }
}

struct Watcher {
    watch: Watch,
    settings: Settings,
    running: Version,
    /// The version last said to be available, so it is said once.
    said: Option<Version>,
    /// Why the last unattended install could not begin, so a reason is said once.
    held: Option<String>,
    /// The factory has been idle since, by the watcher's own reckoning.
    idle_since: Option<Instant>,
    /// A version an unattended install was asked for, and when it may be asked again.
    tried: Option<(Version, Timestamp)>,
}

impl Watcher {
    fn new(watch: Watch, settings: Settings) -> Self {
        Self {
            watch,
            settings,
            running: Version::running(),
            said: None,
            held: None,
            idle_since: None,
            tried: None,
        }
    }

    fn run(mut self, announce: &dyn Fn(String)) -> ! {
        loop {
            self.pass(announce);
            std::thread::sleep(WAKE);
        }
    }

    fn pass(&mut self, announce: &dyn Fn(String)) {
        let config = self.watch.config;
        let now = Timestamp::now();
        let last = layover_update::state::last_check(&self.settings.dir, config.channel);
        let checked = layover_update::due(last.as_ref(), config.check_every(), now)
            // A failure is recorded and retried; it never stops the Tower.
            && layover_update::check(&self.settings, config.channel, now).is_ok();
        let Some(available) =
            layover_update::status(&self.settings, &config, &self.running).available
        else {
            return;
        };
        if self.said.as_ref() != Some(&available.release.version) {
            announce(format!(
                "Layover {} is available ({}). `layover update check` says what changes.",
                available.release.version, available.kind
            ));
            self.said = Some(available.release.version.clone());
        }
        let readies = matches!(config.mode, Mode::Stage | Mode::Auto);
        if checked
            && readies
            && let Some((staged, gate)) = crate::staging::ready(
                &self.settings,
                &available.release,
                &self.watch.factory,
                config.mode == Mode::Auto && !config.require_provenance,
            )
        {
            for line in crate::staging::describe(&staged, Some(&gate)).lines() {
                announce(line.to_owned());
            }
        }
        self.note_idle();
        if config.mode == Mode::Auto {
            self.consider(&available, now, announce);
        }
    }

    /// Keeps count of how long nothing has run, started, been parked or been queued.
    fn note_idle(&mut self) {
        if self.watch.control.is_quiescent() && self.watch.control.nothing_queued() {
            self.idle_since.get_or_insert_with(Instant::now);
        } else {
            self.idle_since = None;
        }
    }

    /// Asks the Tower to install `available` unattended, when the policy and the moment allow.
    fn consider(&mut self, available: &Available, now: Timestamp, announce: &dyn Fn(String)) {
        let state = &self.watch.state;
        let busy = handover::request(state).is_some_and(|request| request.outcome.is_none())
            || layover_store::stop::pending(state).is_some();
        let version = &available.release.version;
        let again = self
            .tried
            .as_ref()
            .is_some_and(|(tried, after)| tried == version && now < *after);
        if busy || again {
            return;
        }
        let staged = layover_update::staged(&self.settings, version);
        let gate = layover_update::gate::recorded(&self.watch.factory);
        let verdict = unattended(
            &self.watch.config,
            available,
            staged.as_ref(),
            gate.as_ref(),
            now,
        );
        let why = match verdict {
            Verdict::Install => None,
            Verdict::NotYet(why) => Some(why),
            Verdict::Person(why) => Some(format!("{why}; `layover update apply` installs it")),
        };
        if let Some(why) = why {
            if self.held.as_ref() != Some(&why) {
                announce(format!("Not installing unattended yet: {why}."));
                self.held = Some(why);
            }
            return;
        }
        let Some((until, retry_after)) = self.moment(now) else {
            return;
        };
        let request = Request {
            version: version.clone(),
            rollback: false,
            now: false,
            including_manual: false,
            by: "[update] mode = \"auto\"".to_owned(),
            at: now,
            progress: Vec::new(),
            outcome: None,
            until,
        };
        if handover::put_request(state, &request).is_ok() {
            announce(format!("Installing {version} unattended."));
            self.tried = Some((version.clone(), retry_after));
            self.held = None;
        }
    }

    /// Whether now is the moment: in the window — with when it closes — or idle long enough. Says
    /// too when to try again if this attempt does not happen.
    fn moment(&self, now: Timestamp) -> Option<(Option<Timestamp>, Timestamp)> {
        let config = &self.watch.config;
        let later = |after: Duration| {
            now.checked_add(
                jiff::SignedDuration::try_from(after).unwrap_or(jiff::SignedDuration::MAX),
            )
            .unwrap_or(Timestamp::MAX)
        };
        if let Some(window) = config.window {
            let here = now.to_zoned(jiff::tz::TimeZone::system());
            let closes = window.open_until(&here)?;
            // Once a window: one that closes before the install happens waits for the next.
            return Some((Some(closes), closes));
        }
        let idle = self
            .idle_since
            .is_some_and(|since| since.elapsed() >= config.idle_for());
        idle.then(|| (None, later(AGAIN_AFTER.max(config.idle_for()))))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use layover_core::config::Config;
    use layover_store::Journal;
    use layover_tower::Factory;
    use layover_update::{
        Attestation, Gate, Kind, LastCheck, Release, Source, Staged, UpdateWindow, state,
    };

    use super::*;
    use crate::tower::Tower;

    struct Scene {
        root: PathBuf,
        tower: Option<Tower>,
        next: Version,
    }

    impl Drop for Scene {
        fn drop(&mut self) {
            drop(self.tower.take());
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// A Tower with nothing to do, and the next minor release found, staged, verified and gated.
    fn scene(tag: &str, kind: Kind) -> Scene {
        let root =
            std::env::temp_dir().join(format!("layover-watching-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("creates");
        let text = "[runners.r]\ncommand = [\"echo\"]\n[agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n";
        std::fs::write(root.join("layover.toml"), text).expect("writes");
        let config: Config = toml::from_str(text).expect("parses");
        let journal =
            Arc::new(Journal::open(root.join(".layover").join("journal")).expect("opens"));
        let factory = Factory::new(config, &root).expect("opens");
        let tower = Tower::start(factory, journal, |_| {}).expect("starts");

        let mut next = Version::running();
        match kind {
            Kind::Major => {
                next.major += 1;
                next.minor = 0;
            }
            _ => next.minor += 1,
        }
        next.patch = 0;
        next.pre.clear();
        let dir = root.join("machine");
        let long_ago: Timestamp = "2026-01-01T00:00:00Z".parse().expect("valid");
        state::record(
            &dir,
            &LastCheck {
                channel: layover_update::Channel::Stable,
                attempted_at: Timestamp::now(),
                succeeded_at: Some(Timestamp::now()),
                error: None,
                failures: 0,
                newest: Some(Release {
                    version: next.clone(),
                    tag: format!("v{next}"),
                    prerelease: false,
                    title: None,
                    notes: String::new(),
                    published_at: Some(long_ago),
                    page: String::new(),
                }),
                etag: None,
            },
        )
        .expect("records");
        let staged = Staged {
            version: next.clone(),
            tag: format!("v{next}"),
            at: long_ago,
            archive: "a.zip".to_owned(),
            sha256: Some("0".repeat(64)),
            attestation: Some(Attestation::Verified {
                signer: "release.yml".to_owned(),
            }),
            trusted_checksum_only: false,
            binary: Some(root.join("layover")),
            about: Some(layover_update::About::of_this_binary(1, "x")),
            crosses_layout: false,
            refused: None,
        };
        let staging = dir.join("staging").join(next.to_string());
        std::fs::create_dir_all(&staging).expect("creates");
        std::fs::write(
            staging.join("staged.json"),
            serde_json::to_string(&staged).expect("serialises"),
        )
        .expect("writes");
        layover_update::gate::record(&Gate {
            version: next.clone(),
            factory: root.join("layover.toml"),
            at: long_ago,
            not_run: None,
            errors: Vec::new(),
            new_warnings: Vec::new(),
        })
        .expect("records");
        Scene {
            root,
            tower: Some(tower),
            next,
        }
    }

    fn watcher(scene: &Scene, config: UpdateConfig) -> Watcher {
        let settings = Settings {
            switched_off: false,
            source: Source::mirror("http://127.0.0.1:9"),
            dir: scene.root.join("machine"),
        };
        Watcher::new(
            Watch {
                config,
                factory: scene.root.join("layover.toml"),
                state: scene.root.join(".layover"),
                control: scene.tower.as_ref().expect("running").control(),
            },
            settings,
        )
    }

    fn auto() -> UpdateConfig {
        UpdateConfig {
            mode: Mode::Auto,
            idle_for_sec: 1,
            ..UpdateConfig::default()
        }
    }

    fn said() -> (Arc<Mutex<Vec<String>>>, impl Fn(String)) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::clone(&lines);
        (lines, move |line| keep.lock().expect("lock").push(line))
    }

    #[test]
    fn an_idle_factory_is_asked_to_install_once() {
        let scene = scene("idle", Kind::Minor);
        let mut watcher = watcher(&scene, auto());
        let (lines, announce) = said();
        let state = scene.root.join(".layover");

        watcher.pass(&announce);
        assert!(
            handover::request(&state).is_none(),
            "not idle long enough yet"
        );

        std::thread::sleep(Duration::from_millis(1_100));
        watcher.pass(&announce);
        let request = handover::request(&state).expect("asked");
        assert_eq!(request.version, scene.next);
        assert_eq!(request.by, "[update] mode = \"auto\"");
        assert_eq!(request.until, None);
        assert!(
            lines
                .lock()
                .expect("lock")
                .iter()
                .any(|line| line.contains("unattended")),
            "{lines:?}"
        );

        // The Tower has it now; it is not asked twice.
        handover::progress(&state, "postponed", Some("postponed"));
        watcher.pass(&announce);
        assert_eq!(
            handover::request(&state)
                .and_then(|request| request.outcome)
                .as_deref(),
            Some("postponed"),
            "an attempt that did not happen waits before it is asked again"
        );
    }

    #[test]
    fn a_window_asks_only_while_it_is_open_and_says_when_it_closes() {
        let scene = scene("window", Kind::Minor);
        let state = scene.root.join(".layover");
        let (_, announce) = said();
        let here = Timestamp::now().to_zoned(jiff::tz::TimeZone::system());
        let at = |minutes: i64| {
            let time = here
                .checked_add(jiff::SignedDuration::from_mins(minutes))
                .expect("in range");
            format!("{:02}:{:02}", time.hour(), time.minute())
        };

        let closed = UpdateWindow::parse(&format!("{}-{}", at(120), at(180))).expect("reads");
        let mut watcher = watcher(
            &scene,
            UpdateConfig {
                window: Some(closed),
                ..auto()
            },
        );
        watcher.pass(&announce);
        assert!(handover::request(&state).is_none(), "the window is closed");

        let open = UpdateWindow::parse(&format!("{}-{}", at(-30), at(90))).expect("reads");
        let mut watcher = self::watcher(
            &scene,
            UpdateConfig {
                window: Some(open),
                ..auto()
            },
        );
        watcher.pass(&announce);
        let request = handover::request(&state).expect("asked");
        let until = request.until.expect("says when the window closes");
        assert!(until > Timestamp::now(), "{until}");
    }

    #[test]
    fn a_major_release_waits_for_a_person_and_says_so_once() {
        let scene = scene("major", Kind::Major);
        let mut watcher = watcher(&scene, auto());
        let (lines, announce) = said();

        watcher.pass(&announce);
        std::thread::sleep(Duration::from_millis(1_100));
        watcher.pass(&announce);
        watcher.pass(&announce);

        assert!(handover::request(&scene.root.join(".layover")).is_none());
        let held: Vec<String> = lines
            .lock()
            .expect("lock")
            .iter()
            .filter(|line| line.starts_with("Not installing"))
            .cloned()
            .collect();
        assert_eq!(held.len(), 1, "{held:?}");
    }
}
