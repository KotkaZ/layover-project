//! Running a program and waiting a bounded time for what it says: a staged binary asked what it
//! is, or `gh` asked whether an archive is what GitHub built.
//!
//! A staged binary has been checked against its release but not yet trusted, so it runs with an
//! environment stripped to what a process needs to start — no tokens, no credentials — in its own
//! staging directory, with `LAYOVER_UPDATE=off` so it cannot itself reach out, and is killed if it
//! has not answered in time.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// What a binary says it is, from `layover about --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct About {
    /// Its version.
    pub version: String,
    /// The platform it was built for.
    pub target: String,
    /// The layout of the state directory it reads and writes.
    pub layout: u32,
    /// The HTTP API it serves.
    pub api: String,
    /// The oldest release it can be updated from in one step.
    pub min_upgrade_from: String,
}

impl About {
    /// This binary, given what the crates above this one know: the state layout and the API.
    #[must_use]
    pub fn of_this_binary(layout: u32, api: &str) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            target: crate::TARGET.to_owned(),
            layout,
            api: api.to_owned(),
            min_upgrade_from: crate::MIN_UPGRADE_FROM.to_owned(),
        }
    }
}

/// What a program printed, and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    /// Its exit code; `None` when a signal ended it.
    pub code: Option<i32>,
    /// What it printed.
    pub stdout: String,
    /// What it complained about.
    pub stderr: String,
}

/// Why a program could not be asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// It is not there.
    #[error("{0} was not found")]
    NotFound(String),
    /// It could not be started.
    #[error("could not start {program}: {reason}")]
    Spawn {
        /// What was started.
        program: String,
        /// Why it could not be.
        reason: String,
    },
    /// It did not answer in time, and was killed.
    #[error("{program} did not answer within {seconds} s")]
    TimedOut {
        /// What was started.
        program: String,
        /// How long it had.
        seconds: u64,
    },
}

/// The variables a process needs to start and find its temporary directory, and nothing else.
const KEPT: &[&str] = &[
    "PATH",
    "SystemRoot",
    "SYSTEMROOT",
    "windir",
    "TEMP",
    "TMP",
    "TMPDIR",
    "HOME",
    "USERPROFILE",
];

/// How a program's environment is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Stripped to what a process needs to start, for a binary not yet trusted.
    Scrubbed,
    /// This process's own, for a tool such as `gh` that needs its configuration.
    Inherited,
}

/// Runs `program` with `args` in `dir`, and waits up to `limit` for it to finish.
///
/// # Errors
///
/// Returns a [`RunError`] when it is missing, cannot start, or outlives `limit`.
pub fn run(
    program: &Path,
    args: &[&str],
    dir: &Path,
    environment: Environment,
    limit: Duration,
) -> Result<Ran, RunError> {
    let name = program.display().to_string();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if environment == Environment::Scrubbed {
        command.env_clear();
        for key in KEPT {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("LAYOVER_UPDATE", "off");
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RunError::NotFound(name.clone())
        } else {
            RunError::Spawn {
                program: name.clone(),
                reason: error.to_string(),
            }
        }
    })?;

    // Read on their own threads, so a program that fills a pipe is not blocked by nobody reading.
    let reader = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.by_ref().take(4 * 1024 * 1024).read_to_end(&mut text);
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let stdout = reader(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = reader(child.stderr.take().map(|pipe| Box::new(pipe) as _));

    let began = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if began.elapsed() < limit => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::TimedOut {
                    program: name,
                    seconds: limit.as_secs(),
                });
            }
        }
    };
    Ok(Ran {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Asks a staged binary what it is.
///
/// # Errors
///
/// Returns why it could not be asked or did not answer with what `about --json` prints.
pub fn about(binary: &Path, dir: &Path) -> Result<About, String> {
    let ran = run(
        binary,
        &["about", "--json"],
        dir,
        Environment::Scrubbed,
        Duration::from_secs(20),
    )
    .map_err(|error| error.to_string())?;
    if ran.code != Some(0) {
        return Err(format!(
            "it exited {}: {}",
            ran.code
                .map_or_else(|| "on a signal".to_owned(), |code| code.to_string()),
            ran.stderr.trim()
        ));
    }
    serde_json::from_str(ran.stdout.trim())
        .map_err(|error| format!("it said something else: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_is_not_there_is_said_to_be_missing() {
        let missing = Path::new("layover-no-such-program-anywhere");
        let error = run(
            missing,
            &[],
            &std::env::temp_dir(),
            Environment::Scrubbed,
            Duration::from_secs(5),
        )
        .expect_err("missing");
        assert!(matches!(error, RunError::NotFound(_)), "{error}");
    }

    #[test]
    fn a_program_that_does_not_answer_in_time_is_killed() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/c", "ping -n 30 127.0.0.1 >nul"])
        } else {
            ("sleep", &["30"])
        };
        let began = Instant::now();
        let error = run(
            Path::new(program),
            args,
            &std::env::temp_dir(),
            Environment::Scrubbed,
            Duration::from_millis(500),
        )
        .expect_err("too slow");

        assert!(matches!(error, RunError::TimedOut { .. }), "{error}");
        assert!(began.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_scrubbed_environment_keeps_no_secrets_and_switches_updates_off() {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/c", "set"])
        } else {
            ("env", &[])
        };
        // Whatever else the test runner's environment holds — tokens included — must be absent.
        let ran = run(
            Path::new(program),
            args,
            &std::env::temp_dir(),
            Environment::Scrubbed,
            Duration::from_secs(10),
        )
        .expect("runs");
        let names: Vec<String> = ran
            .stdout
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_owned()))
            .collect();
        assert!(
            names.iter().any(|name| name == "LAYOVER_UPDATE"),
            "{names:?}"
        );
        let windows_own = ["COMSPEC", "PATHEXT", "PROMPT"];
        for name in &names {
            assert!(
                KEPT.iter().any(|kept| kept.eq_ignore_ascii_case(name))
                    || name == "LAYOVER_UPDATE"
                    || windows_own.iter().any(|own| own.eq_ignore_ascii_case(name))
                    || name == "PWD"
                    || name == "SHLVL"
                    || name == "_",
                "{name} reached a binary that is not trusted yet"
            );
        }
    }
}
