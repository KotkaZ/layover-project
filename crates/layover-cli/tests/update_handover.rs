//! A Tower installing a release into itself, end to end: it drains, swaps its binary, starts the
//! new one at its own address and exits — or, when the new one cannot start, puts its own binary
//! back and carries on serving.
//!
//! The release is this build's own binary, staged under its own version: what is under test is the
//! handover, and a second build of Layover with another version would cost minutes for nothing more.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use layover_update::handover::{self, Request};
use layover_update::probe::spawn_retrying;
use layover_update::{About, Attestation, Staged, Version};

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("layover-handover-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        for dir in ["bin", "factory", "machine", "staged"] {
            std::fs::create_dir_all(path.join(dir)).expect("creates");
        }
        Self(path)
    }

    fn binary(&self) -> PathBuf {
        self.0.join("bin").join(name())
    }

    fn factory(&self) -> PathBuf {
        self.0.join("factory").join("layover.toml")
    }

    fn state(&self) -> PathBuf {
        self.0.join("factory").join(".layover")
    }

    /// Where `Settings::from_env` puts the machine's update state, given this test's environment.
    fn updates(&self) -> PathBuf {
        let machine = self.0.join("machine");
        if cfg!(target_os = "macos") {
            machine.join("Library").join("Application Support")
        } else {
            machine
        }
        .join("layover")
        .join("updates")
    }

    fn command(&self, args: &[&str]) -> Command {
        let machine = self.0.join("machine");
        let mut command = Command::new(self.binary());
        command
            .args(args)
            .current_dir(self.0.join("factory"))
            .env("LAYOVER_UPDATE", "off")
            .env_remove("LAYOVER_SUPERVISOR")
            // systemd's mark of a service, which a CI runner's own service may pass down.
            .env_remove("INVOCATION_ID")
            .env("LOCALAPPDATA", &machine)
            .env("XDG_STATE_HOME", &machine)
            .env("HOME", &machine);
        command
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        // A Tower handed over to is no child of this test: if one outlived a failed assertion,
        // it is found by the pid the handover recorded, and ended.
        let started = handover::request(&self.state())
            .map(|request| request.progress)
            .unwrap_or_default()
            .iter()
            .find_map(|line| {
                let (_, rest) = line.split_once("(pid ")?;
                rest.split(')').next()?.parse::<u32>().ok()
            });
        if let Some(pid) = started {
            let _ = if cfg!(windows) {
                Command::new("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
            } else {
                Command::new("kill").args(["-9", &pid.to_string()]).status()
            };
        }
        for _ in 0..20 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

fn name() -> &'static str {
    if cfg!(windows) {
        "layover.exe"
    } else {
        "layover"
    }
}

fn version() -> Version {
    Version::running()
}

/// A factory with nothing scheduled, so the drain is quiet at once.
fn factory(temp: &Temp) {
    std::fs::copy(env!("CARGO_BIN_EXE_layover"), temp.binary()).expect("copies the binary");
    std::fs::write(
        temp.factory(),
        "[layover]\nwork_dir = \".\"\n[runners.r]\ncommand = [\"echo\"]\n\
         [agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n",
    )
    .expect("writes");
}

/// Stages `binary` as this version's release, verified, as `layover update stage` would leave it.
fn staged(temp: &Temp, binary: &Path) {
    let dir = temp.updates().join("staging").join(version().to_string());
    std::fs::create_dir_all(&dir).expect("creates");
    let copy = temp.0.join("staged").join(name());
    std::fs::copy(binary, &copy).expect("copies");
    let record = Staged {
        version: version(),
        tag: format!("v{}", version()),
        at: jiff::Timestamp::now(),
        archive: "archive".to_owned(),
        sha256: Some("0".repeat(64)),
        attestation: Some(Attestation::Verified {
            signer: "release.yml".to_owned(),
        }),
        trusted_checksum_only: false,
        binary: Some(copy),
        about: Some(About::of_this_binary(
            layover_store::layout::CURRENT,
            env!("CARGO_PKG_VERSION"),
        )),
        crosses_layout: false,
        refused: None,
    };
    std::fs::write(
        dir.join("staged.json"),
        serde_json::to_string(&record).expect("serialises"),
    )
    .expect("writes");
}

/// Starts `serve` from the copied binary, and returns it with the address it bound.
fn serve(temp: &Temp) -> (Child, String) {
    serve_under(temp, None)
}

/// Starts `serve` as a service manager named `supervisor` would, if one is named.
fn serve_under(temp: &Temp, supervisor: Option<&str>) -> (Child, String) {
    let mut command = temp.command(&[
        "serve",
        "--config",
        "layover.toml",
        "--addr",
        "127.0.0.1:0",
        "--no-auth",
    ]);
    if let Some(supervisor) = supervisor {
        command.env("LAYOVER_SUPERVISOR", supervisor);
    }
    // Retried: on Linux the binary this test just copied can read as busy for a moment.
    let mut child =
        spawn_retrying(command.stdout(Stdio::piped()).stderr(Stdio::null())).expect("serve starts");
    let stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let started = Instant::now();
    let mut address = None;
    while address.is_none() && started.elapsed() < Duration::from_secs(30) {
        if let Ok(line) = rx.recv_timeout(Duration::from_secs(1)) {
            address = line
                .strip_prefix("Layover dashboard on http://")
                .map(|address| address.trim_end_matches('/').to_owned());
        }
    }
    if let Some(address) = address {
        return (child, address);
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("serve did not say where it listens");
}

/// The version `/health` at `address` reports, if it answers.
fn health(address: &str) -> Option<String> {
    let mut stream = TcpStream::connect(address).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut text = String::new();
    stream.read_to_string(&mut text).ok()?;
    let body = text.split("\r\n\r\n").nth(1)?;
    let health: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    health["version"].as_str().map(str::to_owned)
}

fn ask(temp: &Temp) {
    handover::put_request(
        &temp.state(),
        &Request {
            version: version(),
            rollback: false,
            now: false,
            including_manual: false,
            by: "test".to_owned(),
            at: jiff::Timestamp::now(),
            progress: Vec::new(),
            outcome: None,
            until: None,
        },
    )
    .expect("asks");
}

fn until<T>(what: &str, within: Duration, mut found: impl FnMut() -> Option<T>) -> T {
    let started = Instant::now();
    while started.elapsed() < within {
        if let Some(value) = found() {
            return value;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("{what} did not happen within {within:?}");
}

/// Stops whatever Tower serves the factory now, and waits for its address to fall silent.
fn stop(temp: &Temp, address: &str) {
    let _ = spawn_retrying(
        temp.command(&["stop", "--config", "layover.toml"])
            .stdout(Stdio::null()),
    )
    .and_then(|mut child| child.wait());
    until("the Tower stopping", Duration::from_secs(30), || {
        health(address).is_none().then_some(())
    });
}

#[test]
fn a_tower_hands_over_to_the_release_it_installed_at_its_own_address() {
    let temp = Temp::new("in-place");
    factory(&temp);
    let (mut old, address) = serve(&temp);
    let before = std::fs::read(temp.binary()).expect("reads");
    staged(&temp, Path::new(env!("CARGO_BIN_EXE_layover")));

    ask(&temp);

    let status = until("the old Tower exiting", Duration::from_secs(90), || {
        old.try_wait().expect("asks")
    });
    let request = handover::request(&temp.state()).expect("kept");
    assert!(status.success(), "{status:?}: {:?}", request.progress);
    assert_eq!(
        request.outcome.as_deref(),
        Some("applied"),
        "{:?}",
        request.progress
    );
    assert_eq!(
        health(&address).as_deref(),
        Some(env!("CARGO_PKG_VERSION")),
        "the new Tower answers where the old one did"
    );

    let record = until(
        "the new Tower proving itself",
        Duration::from_secs(60),
        || handover::handover(&temp.state()).filter(|record| record.healthy_at.is_some()),
    );
    assert_eq!(record.boots, 1);
    assert_eq!(std::fs::read(&record.previous).expect("kept"), before);
    let events: Vec<String> = layover_update::log::read(&temp.state())
        .into_iter()
        .map(|event| event.what)
        .collect();
    assert!(events.contains(&"applied".to_owned()), "{events:?}");
    assert!(events.contains(&"healthy".to_owned()), "{events:?}");

    stop(&temp, &address);
}

#[test]
fn a_release_that_cannot_start_is_rolled_back_by_the_tower_it_was_to_replace() {
    let temp = Temp::new("cannot-start");
    factory(&temp);
    let (mut old, address) = serve(&temp);
    let before = std::fs::read(temp.binary()).expect("reads");
    // A "release" that is not a Tower: it refuses `serve`'s arguments and exits at once.
    let impostor = if cfg!(windows) {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32")
            .join("PING.EXE")
    } else {
        PathBuf::from("/bin/sleep")
    };
    staged(&temp, &impostor);

    ask(&temp);

    let request = until("the install failing", Duration::from_secs(90), || {
        handover::request(&temp.state()).filter(|request| request.outcome.is_some())
    });
    assert_eq!(
        request.outcome.as_deref(),
        Some("failed"),
        "{:?}",
        request.progress
    );
    assert!(
        request
            .progress
            .iter()
            .any(|line| line.contains("exited") && line.contains("before it answered")),
        "{:?}",
        request.progress
    );
    assert!(
        old.try_wait().expect("asks").is_none(),
        "the old Tower carries on"
    );
    until(
        "the old Tower serving again",
        Duration::from_secs(30),
        || health(&address).filter(|said| said == env!("CARGO_PKG_VERSION")),
    );
    assert_eq!(
        std::fs::read(temp.binary()).expect("reads"),
        before,
        "put back"
    );
    assert!(
        layover_update::state::skipped(&temp.updates()).contains(&version()),
        "not offered again"
    );

    stop(&temp, &address);
    let _ = old.wait();
}

#[test]
fn with_no_tower_serving_a_rollback_puts_the_kept_binary_back_at_once() {
    let temp = Temp::new("rollback");
    factory(&temp);
    let previous = temp.0.join("bin").join("layover-1.0.0.old");
    std::fs::write(&previous, b"the version before").expect("writes");
    handover::put_handover(
        &temp.state(),
        &layover_update::Handover {
            from: "1.0.0".parse().expect("valid"),
            to: version(),
            binary: temp.binary(),
            previous: previous.clone(),
            started: jiff::Timestamp::now(),
            healthy_by: jiff::Timestamp::now(),
            boots: 1,
            healthy_at: Some(jiff::Timestamp::now()),
            rolled_back_at: None,
            token: None,
            held: Vec::new(),
        },
    )
    .expect("writes");

    // A copy to read the log with afterwards: the command replaces its own binary, as an
    // installed one would, and what it puts back here is not a program.
    let runner = temp.0.join("staged").join(name());
    std::fs::copy(temp.binary(), &runner).expect("copies");
    let mut command = temp.command(&["update", "rollback", "--config", "layover.toml"]);
    let output = spawn_retrying(command.stdout(Stdio::piped()).stderr(Stdio::piped()))
        .and_then(Child::wait_with_output)
        .expect("runs");
    let said = String::from_utf8_lossy(&output.stdout).into_owned();

    assert_eq!(output.status.code(), Some(5), "{said}");
    assert!(said.contains("Rolled back"), "{said}");
    assert_eq!(
        std::fs::read(temp.binary()).expect("reads"),
        b"the version before"
    );
    assert!(layover_update::state::skipped(&temp.updates()).contains(&version()));

    let log = spawn_retrying(
        Command::new(&runner)
            .args(["update", "log", "--config", "layover.toml"])
            .current_dir(temp.0.join("factory"))
            .stdout(Stdio::piped()),
    )
    .and_then(Child::wait_with_output)
    .expect("runs");
    let text = String::from_utf8_lossy(&log.stdout);
    assert!(
        text.contains(&format!("rolled_back {} → 1.0.0", version())),
        "{text}"
    );
}

/// Under systemd or launchd the old Tower exits 75 and the service manager starts the new release;
/// on Windows there is no such path, and the Tower always hands over in place.
#[cfg(unix)]
#[test]
fn under_a_supervisor_the_tower_exits_75_for_it_to_start_the_new_release() {
    let temp = Temp::new("supervised");
    factory(&temp);
    let (mut old, _) = serve_under(&temp, Some("test"));
    staged(&temp, Path::new(env!("CARGO_BIN_EXE_layover")));

    ask(&temp);

    let status = until("the old Tower exiting", Duration::from_secs(90), || {
        old.try_wait().expect("asks")
    });
    let request = handover::request(&temp.state()).expect("kept");
    assert_eq!(status.code(), Some(75), "{:?}", request.progress);
    assert!(
        request
            .progress
            .iter()
            .any(|line| line.contains("Exiting 75")),
        "{:?}",
        request.progress
    );

    // The supervisor's part: start the same command again.
    let (mut new, address) = serve_under(&temp, Some("test"));
    let record = until(
        "the new Tower proving itself",
        Duration::from_secs(60),
        || handover::handover(&temp.state()).filter(|record| record.healthy_at.is_some()),
    );
    assert_eq!(record.boots, 1);
    assert_eq!(
        handover::request(&temp.state())
            .and_then(|request| request.outcome)
            .as_deref(),
        Some("applied")
    );

    stop(&temp, &address);
    let _ = new.wait();
}

#[test]
fn an_unattended_install_whose_window_closes_while_a_run_goes_on_is_postponed() {
    let temp = Temp::new("window-closes");
    factory(&temp);
    let sleeps = if cfg!(windows) {
        r#"["cmd", "/c", "ping -n 9 127.0.0.1 >nul"]"#
    } else {
        r#"["sh", "-c", "sleep 8"]"#
    };
    std::fs::write(
        temp.factory(),
        format!(
            "[layover]\nwork_dir = \".\"\n[runners.r]\ncommand = {sleeps}\n\
             [agents.a]\nprompt = \"p\"\nrunner = \"r\"\nentry = true\n"
        ),
    )
    .expect("writes");
    let (mut old, address) = serve(&temp);
    staged(&temp, Path::new(env!("CARGO_BIN_EXE_layover")));
    let journal = layover_store::Journal::open(temp.state().join("journal")).expect("opens");
    journal
        .queue(layover_core::queue::Queued::new(
            layover_core::flight::Flight::new(
                layover_core::flight::ItineraryId::generate(),
                layover_core::flight::Origin::Human,
                layover_core::agent::AgentName::new("a"),
                "go",
                4,
            ),
            None,
            std::collections::BTreeMap::new(),
        ))
        .expect("queues");
    until("the run starting", Duration::from_secs(30), || {
        std::fs::read_dir(temp.state().join("state").join("runs"))
            .ok()?
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .then_some(())
    });

    let closes = jiff::Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(2))
        .expect("in range");
    handover::put_request(
        &temp.state(),
        &Request {
            version: version(),
            rollback: false,
            now: false,
            including_manual: false,
            by: "[update] mode = \"auto\"".to_owned(),
            at: jiff::Timestamp::now(),
            progress: Vec::new(),
            outcome: None,
            until: Some(closes),
        },
    )
    .expect("asks");

    let request = until(
        "the install being postponed",
        Duration::from_secs(20),
        || handover::request(&temp.state()).filter(|request| request.outcome.is_some()),
    );
    assert_eq!(
        request.outcome.as_deref(),
        Some("postponed"),
        "{:?}",
        request.progress
    );
    assert!(
        request
            .progress
            .iter()
            .any(|line| line.contains("window closed")),
        "{:?}",
        request.progress
    );
    assert!(
        old.try_wait().expect("asks").is_none(),
        "the Tower carries on"
    );
    assert_eq!(health(&address).as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(
        layover_update::log::read(&temp.state())
            .iter()
            .any(|event| event.what == "postponed")
    );

    stop(&temp, &address);
    let _ = old.wait();
}

/// The Tower a handover started finds the request still open — under a service manager the old
/// Tower has exited without seeing it answer. It ends the request once it is healthy, and does not
/// take the request for its own version as one to install.
#[test]
fn a_tower_started_for_an_install_ends_it_when_healthy_and_does_not_install_itself_again() {
    let temp = Temp::new("arrived");
    factory(&temp);
    // As a handover leaves them: the request, then the record of the handover it began.
    ask(&temp);
    let now = jiff::Timestamp::now();
    handover::put_handover(
        &temp.state(),
        &layover_update::Handover {
            from: "1.0.0".parse().expect("valid"),
            to: version(),
            binary: temp.binary(),
            previous: temp.0.join("bin").join("layover-1.0.0.old"),
            started: now,
            healthy_by: now
                .checked_add(jiff::SignedDuration::from_mins(10))
                .expect("in range"),
            boots: 0,
            healthy_at: None,
            rolled_back_at: None,
            token: None,
            held: Vec::new(),
        },
    )
    .expect("writes");

    let (mut tower, address) = serve(&temp);
    let request = until("the request ending", Duration::from_secs(60), || {
        handover::request(&temp.state()).filter(|request| request.outcome.is_some())
    });

    assert_eq!(
        request.outcome.as_deref(),
        Some("applied"),
        "{:?}",
        request.progress
    );
    assert!(
        request
            .progress
            .iter()
            .any(|line| line.contains("it is healthy")),
        "{:?}",
        request.progress
    );
    assert!(
        !request
            .progress
            .iter()
            .any(|line| line.starts_with("Installing")),
        "it did not drain to install itself: {:?}",
        request.progress
    );
    assert!(handover::handover(&temp.state()).is_some_and(|record| record.healthy_at.is_some()));
    assert!(tower.try_wait().expect("asks").is_none());
    assert_eq!(health(&address).as_deref(), Some(env!("CARGO_PKG_VERSION")));

    stop(&temp, &address);
    let _ = tower.wait();
}
