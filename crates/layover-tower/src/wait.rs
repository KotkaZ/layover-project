//! Waiting for a run, and ending one that will not end by itself.
//!
//! # Why a timeout is not automatic recovery
//!
//! Recovery exists for interruptions, where the cause has gone away — the machine restarted, the
//! supervisor died. A timeout means the work was too big or something is wedged, and both of those
//! repeat identically, so retrying spends money to arrive in the same place. The run is killed, it
//! is recorded as `timed_out`, and whoever sent the flight is told.
//!
//! # Why the whole tree is killed
//!
//! An agent CLI is rarely one process. It starts language servers, shells out to `git`, runs test
//! suites. Killing only the process the supervisor spawned leaves those behind — still holding the
//! workspace, still burning CPU, and on the next run, still there. What "kill the tree" means
//! differs sharply by platform, which is why it is here and not inlined.

use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::spawn::{Finished, SpawnError, Started};

/// How often a waiting run is checked.
///
/// Polling rather than blocking, because the wait has to be interruptible: a run being watched for
/// a timeout, or for a Ground Stop, cannot be sitting in an uninterruptible `wait()`. A tenth of a
/// second is far below any timeout worth configuring and costs nothing measurable.
const POLL: Duration = Duration::from_millis(100);

/// Why waiting stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The child exited on its own.
    Exited,
    /// The child outlived `timeout_sec` and was killed.
    TimedOut,
    /// A Ground Stop was engaged and the child was killed.
    Halted,
}

/// Waits for `started`, killing it if it outlives `timeout` or if `should_stop` becomes true.
///
/// `should_stop` is polled rather than passed as a signal so the caller decides what stopping
/// means — a Ground Stop file appearing, an operator request, a shutdown.
///
/// # Errors
///
/// Returns [`SpawnError::Io`] when the child cannot be waited on or killed.
pub fn wait_for(
    mut started: Started,
    timeout: Option<Duration>,
    mut should_stop: impl FnMut() -> bool,
) -> Result<(Finished, Ended), SpawnError> {
    let began = Instant::now();

    loop {
        if let Some(status) = started.try_wait()? {
            return Ok((started.into_finished(status), Ended::Exited));
        }

        if should_stop() {
            kill_tree(started.pid())?;
            let status = started.wait_after_kill()?;
            return Ok((started.into_finished(status), Ended::Halted));
        }

        if timeout.is_some_and(|limit| began.elapsed() >= limit) {
            kill_tree(started.pid())?;
            let status = started.wait_after_kill()?;
            return Ok((started.into_finished(status), Ended::TimedOut));
        }

        std::thread::sleep(POLL);
    }
}

/// Ends a process and everything it started.
///
/// # Errors
///
/// Returns [`SpawnError::Io`] when the platform's terminating command cannot be run. A process
/// that has already exited is not an error: the outcome wanted is that it is gone, and it is.
pub fn kill_tree(pid: u32) -> Result<(), SpawnError> {
    #[cfg(windows)]
    {
        // Windows has no process groups in the Unix sense. `taskkill /T` walks the parent chain
        // the kernel records, which is the only reliable way to reach a CLI's children.
        let status = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(SpawnError::Io)?;

        // 128 means "no such process", which is the state being asked for.
        if status.success() || status.code() == Some(128) {
            return Ok(());
        }

        Err(SpawnError::Io(io::Error::other(format!(
            "taskkill refused to end process tree {pid}: {status}"
        ))))
    }

    #[cfg(not(windows))]
    {
        // Negating the identifier addresses the process group, which is what catches the children.
        let status = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(SpawnError::Io)?;

        if status.success() {
            return Ok(());
        }

        // The group may not exist because the child never made one. Fall back to the process.
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(SpawnError::Io)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn::{Plan, start};
    use layover_core::agent::AgentName;
    use layover_core::config::Runner;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn sleeping(seconds: u32) -> Runner {
        let command = if cfg!(windows) {
            // `timeout` needs a console; ping against loopback is the portable idle.
            format!(
                r#"["cmd", "/c", "ping -n {} 127.0.0.1 > nul"]"#,
                seconds + 1
            )
        } else {
            format!(r#"["sh", "-c", "sleep {seconds}"]"#)
        };
        toml::from_str(&format!("command = {command}")).expect("parses")
    }

    fn quick() -> Runner {
        let command = if cfg!(windows) {
            r#"["cmd", "/c", "exit 0"]"#
        } else {
            r#"["sh", "-c", "exit 0"]"#
        };
        toml::from_str(&format!("command = {command}")).expect("parses")
    }

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("layover-wait-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn plan(temp: &Temp, runner: Runner) -> Plan {
        Plan {
            agent: AgentName::new("tester"),
            runner,
            model: None,
            payload: "go".to_owned(),
            hangar: temp.0.join("hangar"),
            work_dir: temp.0.clone(),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn a_run_that_finishes_on_its_own_is_not_reported_as_killed() {
        let temp = Temp::new("exits");
        let started = start(&plan(&temp, quick())).expect("starts");

        let (finished, ended) =
            wait_for(started, Some(Duration::from_secs(30)), || false).expect("waits");

        assert_eq!(ended, Ended::Exited);
        assert!(finished.succeeded());
    }

    #[test]
    fn a_run_that_outlives_its_timeout_is_killed_and_says_so() {
        let temp = Temp::new("timeout");
        let started = start(&plan(&temp, sleeping(30))).expect("starts");

        let (_, ended) =
            wait_for(started, Some(Duration::from_millis(300)), || false).expect("waits");

        assert_eq!(ended, Ended::TimedOut, "a wedged run has to be endable");
    }

    #[test]
    fn a_stop_request_ends_a_running_child() {
        // This is what makes Ground Stop real rather than advisory: a kill switch that only
        // blocks new work while the expensive thing keeps running is not a kill switch.
        let temp = Temp::new("halt");
        let started = start(&plan(&temp, sleeping(30))).expect("starts");

        let mut checks = 0;
        let (_, ended) = wait_for(started, None, || {
            checks += 1;
            checks > 1
        })
        .expect("waits");

        assert_eq!(ended, Ended::Halted);
    }

    #[test]
    fn no_timeout_means_waiting_indefinitely_not_killing_immediately() {
        let temp = Temp::new("untimed");
        let started = start(&plan(&temp, quick())).expect("starts");

        let (_, ended) = wait_for(started, None, || false).expect("waits");
        assert_eq!(ended, Ended::Exited);
    }

    #[test]
    fn killing_something_already_gone_is_not_an_error() {
        // The wanted outcome is that the process is not running, and it is not.
        let temp = Temp::new("gone");
        let started = start(&plan(&temp, quick())).expect("starts");
        let pid = started.pid();
        let _ = wait_for(started, None, || false).expect("waits");

        kill_tree(pid).expect("killing a finished process is a no-op");
    }
}
