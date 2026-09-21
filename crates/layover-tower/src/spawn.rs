//! Starting one agent CLI and watching it finish.
//!
//! # Why the transcript is a file and not a buffer
//!
//! A run's output is the only account of what an agent actually did, and it is worth money. Held
//! in memory it is lost the moment the supervisor dies — which is the case where it is most needed,
//! because that is the run somebody will have to reconstruct. Streamed to a file it survives, it
//! can be tailed live, and it costs nothing to keep.
//!
//! # Why the environment is built rather than inherited
//!
//! The child gets exactly the variables its agent named in `env_from`, and nothing else. Handing a
//! child the supervisor's whole environment would give the telemetry agent the publishing
//! credentials and the reviewer the cloud keys, so a single prompt injection anywhere would reach
//! all of them. A variable that was named but is not set is an error, not a shrug: an agent
//! starting without the credential it declared will fail somewhere further away, having already
//! cost money.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use jiff::Timestamp;
use layover_core::agent::AgentName;
use layover_core::config::Runner;

/// Everything needed to start one run.
///
/// Assembled by the caller from a factory definition and a flight; this module does not read
/// configuration or decide anything about routing.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Which agent is running.
    pub agent: AgentName,
    /// How to invoke its CLI.
    pub runner: Runner,
    /// The model to pass, when the agent declared one.
    pub model: Option<String>,
    /// The composed payload, from `layover_core::payload::compose`.
    pub payload: String,
    /// Where this run's files go: the payload, the transcript, the state record.
    pub hangar: PathBuf,
    /// The directory the child runs in.
    pub work_dir: PathBuf,
    /// Variables to pass, already resolved from the supervisor's own environment.
    pub env: BTreeMap<String, String>,
    /// The MCP configuration written for this run, when the factory is serving one.
    ///
    /// Held as a path rather than as content because the flag a CLI takes names a file, and the
    /// file has to outlive this struct — it is read by the child, after the spawn.
    pub mcp_config: Option<PathBuf>,
}

/// A process that has been started and recorded.
#[derive(Debug)]
pub struct Started {
    /// The running child.
    child: std::process::Child,
    /// Where its output is being written.
    transcript: PathBuf,
    /// When it began.
    began: Timestamp,
    /// Its process identifier, as recorded before the spawn.
    identifier: u32,
}

impl Started {
    /// The process identifier.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.identifier
    }

    /// Where the child's output is accumulating.
    #[must_use]
    pub fn transcript(&self) -> &Path {
        &self.transcript
    }

    /// When the run began.
    #[must_use]
    pub const fn started_at(&self) -> Timestamp {
        self.began
    }

    /// Whether the child has exited, without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError::Io`] when the child cannot be checked.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, SpawnError> {
        self.child.try_wait().map_err(SpawnError::Io)
    }

    /// Reaps a child that has been killed, so it does not linger as a zombie.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError::Io`] when the child cannot be waited on.
    pub fn wait_after_kill(&mut self) -> Result<std::process::ExitStatus, SpawnError> {
        self.child.wait().map_err(SpawnError::Io)
    }

    /// Turns a finished child into the record of how it ended.
    #[must_use]
    pub fn into_finished(self, status: std::process::ExitStatus) -> Finished {
        Finished {
            exit_code: status.code(),
            finished_at: Timestamp::now(),
            transcript: self.transcript,
            started_at: self.began,
        }
    }

    /// Waits for the child and reports how it ended.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnError::Io`] when the child cannot be waited on.
    pub fn wait(mut self) -> Result<Finished, SpawnError> {
        let status = self.child.wait().map_err(SpawnError::Io)?;
        Ok(self.into_finished(status))
    }
}

/// How a run ended.
#[derive(Debug, Clone)]
pub struct Finished {
    /// The child's exit code, when it had one. `None` means it was killed by a signal.
    pub exit_code: Option<i32>,
    /// When it ended.
    pub finished_at: Timestamp,
    /// Where its output was written.
    pub transcript: PathBuf,
    /// When it began.
    pub started_at: Timestamp,
}

impl Finished {
    /// Whether the child reported success.
    ///
    /// A child killed by a signal has no exit code and did not succeed. Treating the absence of a
    /// code as success would make a killed run indistinguishable from a clean one.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// What can go wrong before a child is running.
#[derive(Debug)]
pub enum SpawnError {
    /// A variable the agent declared is not set in the supervisor's environment.
    MissingEnv {
        /// The variable that is not set.
        name: String,
    },
    /// The runner names no command at all.
    EmptyCommand,
    /// The filesystem or the process refused.
    Io(io::Error),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingEnv { name } => write!(
                f,
                "`{name}` is named in `env_from` but is not set; the run would start without the \
                 credential it declared and fail somewhere further away"
            ),
            Self::EmptyCommand => f.write_str("the runner names no command to run"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for SpawnError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The file a run's composed instructions are written to.
pub const PAYLOAD_FILE: &str = "prompt.md";

/// The file a run's output is streamed to.
pub const TRANSCRIPT_FILE: &str = "transcript.log";

/// Variables a process needs to exist at all, as opposed to variables an agent was given.
///
/// `env_from` isolates *credentials*: the telemetry agent should not hold the publishing token.
/// It was never meant to stop a child finding its own shell. Clearing the environment outright
/// does exactly that — on Windows a command interpreter without `SystemRoot` cannot start, and on
/// any platform a child without `PATH` cannot find the programs it shells out to. The symptom is
/// a run that exits instantly with no useful output, which reads like the agent failing rather
/// than like the supervisor having made it impossible to succeed.
///
/// So the child gets this, and what its agent declared, and nothing else. Nothing here carries a
/// secret; every one of them is a fact about the machine.
///
/// # Why the Windows entries are not optional
///
/// Found by running a real agent CLI on Windows rather than a shell stand-in, which is the only
/// way any of this surfaces:
///
/// - **`PATHEXT`** decides that `git` means `git.exe`. `cmd.exe` falls back to a built-in list
///   when it is unset, so a stand-in invoked through `cmd /c` works and hides the problem
///   entirely — but **PowerShell does not fall back**, and agent CLIs shell out through
///   PowerShell on Windows. Without it every `git`, `npm` or `cargo` an agent runs fails with
///   "not recognized", which reads like a broken machine rather than a stripped environment.
/// - **`USERPROFILE`, `APPDATA`, `LOCALAPPDATA`** are where a Windows program keeps its own
///   configuration and credentials, and are what `~` expands to. `HOME` was already granted for
///   exactly this reason on Unix; omitting the Windows spelling of the same thing made an agent
///   unable to find the login it had already performed.
const BASE_ENV: [&str; 14] = [
    "PATH",
    // Windows: a command interpreter will not start without these two.
    "SystemRoot",
    "COMSPEC",
    // Windows: how a bare `git` is resolved to `git.exe`. PowerShell has no fallback for this.
    "PATHEXT",
    "SystemDrive",
    // Windows: the equivalents of HOME below, and what `~` expands to.
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    // Both: where a process is allowed to write scratch files.
    "TEMP",
    "TMP",
    "TMPDIR",
    // Unix: tools that look up the current user, and anything reading a dotfile.
    "HOME",
    "USER",
    "LOGNAME",
];

/// Builds the environment a child receives: the machine's basics, plus what the agent declared.
///
/// Declared variables win, so a factory that deliberately overrides `PATH` for an agent gets the
/// `PATH` it asked for.
fn environment(declared: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = BASE_ENV
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_owned(), value))
        })
        .collect();

    env.extend(declared.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// Starts the run described by `plan`.
///
/// The payload is written to the hangar first, so that it exists whether or not the runner wants a
/// path to it — a run whose instructions cannot be read afterwards is a run nobody can explain.
///
/// # Errors
///
/// Returns [`SpawnError`] when a declared variable is unset, the command is empty, or the process
/// cannot be started.
pub fn start(plan: &Plan) -> Result<Started, SpawnError> {
    if plan.runner.command.is_empty() {
        return Err(SpawnError::EmptyCommand);
    }

    fs::create_dir_all(&plan.hangar)?;

    // Written whether or not this runner takes a path. The transcript explains what the agent did;
    // only this explains what it was asked.
    let payload_path = plan.hangar.join(PAYLOAD_FILE);
    fs::write(&payload_path, &plan.payload)?;

    let payload_arg = plan
        .runner
        .takes_prompt_path()
        .then(|| payload_path.display().to_string());

    let mcp_arg = plan
        .mcp_config
        .as_ref()
        .map(|path| path.display().to_string());

    let argv = plan.runner.invocation_with_mcp(
        payload_arg.as_deref(),
        plan.model.as_deref(),
        mcp_arg.as_deref(),
    );
    let (program, arguments) = argv.split_first().ok_or(SpawnError::EmptyCommand)?;

    let transcript = plan.hangar.join(TRANSCRIPT_FILE);
    // Both streams into one file, in the order the child produced them. Splitting them makes an
    // error impossible to place against the work that caused it.
    let sink = File::create(&transcript)?;
    let sink_for_stderr = sink.try_clone()?;

    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(&plan.work_dir)
        .env_clear()
        .envs(environment(&plan.env))
        .stdin(Stdio::piped())
        .stdout(Stdio::from(sink))
        .stderr(Stdio::from(sink_for_stderr));

    let started_at = Timestamp::now();
    let mut child = command.spawn()?;

    // The payload goes to stdin even when the runner also took a path: a CLI that reads a file
    // still needs its stdin closed, and one that does not needs the text.
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        // A child that exits before reading closes the pipe, which is not an error here — the
        // run has ended, and how it ended is the exit code's business.
        let _ = stdin.write_all(plan.payload.as_bytes());
    }

    let identifier = child.id();

    Ok(Started {
        child,
        transcript,
        began: started_at,
        identifier,
    })
}

/// Resolves the variables an agent named, from the supervisor's own environment.
///
/// # Errors
///
/// Returns [`SpawnError::MissingEnv`] for the first name that is not set.
pub fn env_from(names: &[String]) -> Result<BTreeMap<String, String>, SpawnError> {
    let mut out = BTreeMap::new();

    for name in names {
        let value =
            std::env::var(name).map_err(|_| SpawnError::MissingEnv { name: name.clone() })?;
        out.insert(name.clone(), value);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command that exists on every platform CI and development run on, so the tests exercise a
    /// real process rather than a mock of one.
    fn echoing(text: &str) -> Runner {
        let command = if cfg!(windows) {
            vec!["cmd".to_owned(), "/c".to_owned(), format!("echo {text}")]
        } else {
            vec!["sh".to_owned(), "-c".to_owned(), format!("echo {text}")]
        };

        toml::from_str(&format!(
            "command = [{}]",
            command
                .iter()
                .map(|a| format!("{a:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .expect("parses")
    }

    fn failing() -> Runner {
        let command = if cfg!(windows) {
            r#"["cmd", "/c", "exit 3"]"#
        } else {
            r#"["sh", "-c", "exit 3"]"#
        };
        toml::from_str(&format!("command = {command}")).expect("parses")
    }

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-tower-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn plan(temp: &Temp, runner: Runner, payload: &str) -> Plan {
        Plan {
            agent: AgentName::new("tester"),
            runner,
            model: None,
            payload: payload.to_owned(),
            hangar: temp.0.join("hangar"),
            work_dir: temp.0.clone(),
            env: BTreeMap::new(),
            mcp_config: None,
        }
    }

    #[test]
    fn a_run_really_starts_a_process_and_reports_how_it_ended() {
        let temp = Temp::new("ok");
        let started = start(&plan(&temp, echoing("hello"), "do the thing")).expect("starts");

        assert!(started.pid() > 0, "a started run has a process identifier");

        let finished = started.wait().expect("waits");
        assert!(finished.succeeded(), "{:?}", finished.exit_code);
        assert!(finished.finished_at >= finished.started_at);
    }

    #[test]
    fn the_transcript_holds_what_the_child_wrote() {
        let temp = Temp::new("transcript");
        let finished = start(&plan(&temp, echoing("marker-42"), "go"))
            .expect("starts")
            .wait()
            .expect("waits");

        let text = fs::read_to_string(&finished.transcript).expect("transcript exists");
        assert!(text.contains("marker-42"), "{text:?}");
    }

    #[test]
    fn the_payload_is_written_where_it_can_be_read_afterwards() {
        // The transcript says what the agent did. Only this says what it was asked, and without it
        // a run that went wrong cannot be explained.
        let temp = Temp::new("payload");
        let asked = plan(&temp, echoing("x"), "You are `tester`.\n\nDo the thing.");
        start(&asked).expect("starts").wait().expect("waits");

        let written = fs::read_to_string(asked.hangar.join(PAYLOAD_FILE)).expect("payload exists");
        assert_eq!(written, asked.payload);
    }

    #[test]
    fn a_non_zero_exit_is_not_a_success() {
        let temp = Temp::new("fail");
        let finished = start(&plan(&temp, failing(), "go"))
            .expect("starts")
            .wait()
            .expect("waits");

        assert_eq!(finished.exit_code, Some(3));
        assert!(!finished.succeeded());
    }

    #[test]
    fn a_declared_variable_that_is_not_set_stops_the_run_before_it_costs_anything() {
        // Starting without a credential the agent declared means failing somewhere further away,
        // after the money has been spent.
        let name = format!("LAYOVER_TEST_ABSENT_{}", std::process::id());
        let error = env_from(std::slice::from_ref(&name)).expect_err("should refuse");

        assert!(matches!(&error, SpawnError::MissingEnv { name: n } if *n == name));
        assert!(error.to_string().contains("env_from"), "{error}");
    }

    #[test]
    fn only_the_named_variables_are_resolved() {
        // Safety-Q25: inheriting the supervisor's environment would hand every agent every other
        // agent's credentials. `PATH` is used because it is always set, so the test needs to
        // mutate no environment of its own.
        let resolved = env_from(&["PATH".to_owned()]).expect("PATH is always set");

        assert_eq!(resolved.len(), 1, "nothing else should come along");
        assert!(resolved.contains_key("PATH"));
    }

    #[test]
    fn nothing_named_means_an_empty_environment_not_an_inherited_one() {
        let resolved = env_from(&[]).expect("resolves");
        assert!(
            resolved.is_empty(),
            "an agent that declared no variables gets none, not all of them"
        );
    }

    #[test]
    fn a_child_gets_enough_environment_to_actually_run() {
        // Found the hard way: clearing the environment outright leaves a child unable to start at
        // all -- on Windows a command interpreter without `SystemRoot` simply exits -- and the
        // symptom reads like the agent failing rather than like the supervisor having made
        // success impossible.
        let base = environment(&BTreeMap::new());

        assert!(
            base.contains_key("PATH"),
            "a child cannot find anything without PATH"
        );
        if cfg!(windows) {
            assert!(
                base.contains_key("SystemRoot"),
                "cmd.exe will not start without SystemRoot"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_child_can_resolve_a_bare_command_through_powershell() {
        // The bug this exists for: an agent shelling out to `git` got "not recognized". PATH was
        // forwarded; PATHEXT was not, and that is what decides `git` means `git.exe`.
        //
        // What makes it worth a test rather than a line in BASE_ENV is that it is invisible to
        // every cheap test. `cmd.exe` falls back to a built-in extension list when PATHEXT is
        // unset, so a shell stand-in invoked through `cmd /c` passes either way. PowerShell has
        // no such fallback, and PowerShell is what agent CLIs shell out through on Windows.
        let base = environment(&BTreeMap::new());

        assert!(
            base.contains_key("PATHEXT"),
            "without PATHEXT, PowerShell cannot resolve `git` to `git.exe`"
        );
        assert!(
            base.contains_key("USERPROFILE"),
            "a CLI looking for its own credentials under `~` needs USERPROFILE on Windows, \
             which is what HOME is for elsewhere"
        );
    }

    #[test]
    fn a_declared_variable_overrides_the_machine_default() {
        let mut declared = BTreeMap::new();
        declared.insert("PATH".to_owned(), "/only/this".to_owned());

        assert_eq!(
            environment(&declared).get("PATH").map(String::as_str),
            Some("/only/this"),
            "a factory that deliberately sets PATH for an agent should get it"
        );
    }

    #[test]
    fn the_base_environment_carries_nothing_secret() {
        // The point of `env_from` is that the telemetry agent does not hold the publishing token.
        // That holds only while the base set stays facts-about-the-machine.
        //
        // The markers are the redactor's, and anchored for the same reason: a bare "PAT" matches
        // "PATH", which is how this test failed the first time it was written.
        for name in BASE_ENV {
            let upper = name.to_ascii_uppercase();
            for marker in [
                "TOKEN",
                "SECRET",
                "PASSWORD",
                "APIKEY",
                "_PAT",
                "CREDENTIAL",
            ] {
                assert!(
                    !upper.contains(marker),
                    "`{name}` looks like a credential and must not be passed by default"
                );
            }
        }
    }

    #[test]
    fn an_empty_command_is_refused_rather_than_panicking() {
        let temp = Temp::new("empty");
        let runner: Runner = toml::from_str("command = []").expect("parses");

        assert!(matches!(
            start(&plan(&temp, runner, "go")),
            Err(SpawnError::EmptyCommand)
        ));
    }
}
