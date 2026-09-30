//! Asking the operating system whether a recorded run is still alive.
//!
//! A Tower that comes back after going away finds records of runs it started and never saw
//! finish. Before it may start a replacement it has to know the old process is gone — starting one
//! beside a process that is still working duplicates whatever it was doing — and before it waits
//! for one it has to know the process under that identifier is the same one: identifiers are
//! reused, and waiting on a stranger that inherited the number is waiting forever.
//!
//! So the question asked is not only "is something running as this pid" but "how long has it been
//! running", which gives its start time to compare with the one recorded. Asked through the
//! platform's own tools — `ps` on Unix, `Get-Process` on Windows — because the workspace forbids
//! `unsafe`, and a restart is rare enough that spawning a short command to answer it costs nothing.

use std::process::{Command, Stdio};

use jiff::{SignedDuration, Timestamp};

use crate::state::Verdict;

/// How far apart the recorded start and the one the system reports may be and still be the same
/// process. The record is written a moment before the spawn and the system reports whole seconds;
/// an identifier reused within this long of the original starting is not a real possibility.
const SAME_PROCESS: SignedDuration = SignedDuration::from_secs(30);

/// What the operating system says about `pid`, measured against when the run was `started_at`.
///
/// `None` when it cannot say: the tool is missing, or refused to answer. The caller must then
/// treat the process as unaccounted for, which is the safe reading — nothing is started beside it.
#[must_use]
pub fn probe(pid: u32, started_at: Timestamp) -> Option<Verdict> {
    let seconds = match alive_for(pid)? {
        Seen::Gone => return Some(Verdict::ConfirmedGone),
        Seen::RunningFor(seconds) => seconds,
    };

    let began = Timestamp::now()
        .checked_sub(SignedDuration::from_secs(
            i64::try_from(seconds).unwrap_or(i64::MAX),
        ))
        .ok()?;
    let apart = began.duration_since(started_at).abs();
    Some(if apart <= SAME_PROCESS {
        Verdict::StillRunning
    } else {
        Verdict::Reused
    })
}

/// What the system reported about a process identifier.
enum Seen {
    /// Nothing is running under it.
    Gone,
    /// Something is, and has been for this many seconds.
    RunningFor(u64),
}

/// What the system says about `pid`, or `None` when it would not say.
fn alive_for(pid: u32) -> Option<Seen> {
    if cfg!(windows) {
        let script = format!(
            "try {{ $p = Get-Process -Id {pid} -ErrorAction Stop; \
             [int64]((Get-Date) - $p.StartTime).TotalSeconds }} \
             catch [Microsoft.PowerShell.Commands.ProcessCommandException] {{ 'gone' }} \
             catch {{ 'unknown' }}"
        );
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let said = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        match said.as_str() {
            "gone" => Some(Seen::Gone),
            other => other.parse::<u64>().ok().map(Seen::RunningFor),
        }
    } else {
        let output = Command::new("ps")
            .args(["-o", "etime=", "-p", &pid.to_string()])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        read_ps(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        )
    }
}

/// What `ps -o etime= -p PID` said.
///
/// Nothing at all is its answer for a pid that is not running. Nothing on stdout with a complaint on
/// stderr is a `ps` that did not understand the question — the one in `BusyBox` has no `-p` — and
/// reading that as "gone" would let recovery start a second run beside one still going.
fn read_ps(stdout: &str, stderr: &str) -> Option<Seen> {
    let said = stdout.trim();
    if said.is_empty() {
        return stderr.trim().is_empty().then_some(Seen::Gone);
    }
    elapsed(said).map(Seen::RunningFor)
}

/// Reads `ps`'s elapsed time, `[[dd-]hh:]mm:ss`, as seconds.
fn elapsed(text: &str) -> Option<u64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, text),
    };
    let parts: Vec<u64> = clock
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    Some(((days * 24 + hours) * 60 + minutes) * 60 + seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeper() -> std::process::Child {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/c", "ping -n 30 127.0.0.1 >nul"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30"]);
            command
        };
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawns")
    }

    #[test]
    fn a_process_that_is_running_and_started_when_recorded_is_still_running() {
        let started_at = Timestamp::now();
        let mut child = sleeper();

        assert_eq!(probe(child.id(), started_at), Some(Verdict::StillRunning));

        let _ = crate::wait::kill_tree(child.id());
        let _ = child.wait();
    }

    #[test]
    fn a_process_that_has_exited_is_confirmed_gone() {
        let started_at = Timestamp::now();
        let mut child = sleeper();
        let pid = child.id();
        let _ = crate::wait::kill_tree(pid);
        let _ = child.wait();

        assert_eq!(probe(pid, started_at), Some(Verdict::ConfirmedGone));
    }

    #[test]
    fn a_live_process_that_started_long_after_the_record_is_a_reused_identifier() {
        // This test process is alive, and it did not start a day ago.
        let a_day_ago = Timestamp::now() - SignedDuration::from_hours(24);

        assert_eq!(probe(std::process::id(), a_day_ago), Some(Verdict::Reused));
    }

    #[test]
    fn a_ps_that_did_not_understand_the_question_is_not_an_answer() {
        assert!(matches!(read_ps("", ""), Some(Seen::Gone)));
        assert!(matches!(
            read_ps("   01:05\n", ""),
            Some(Seen::RunningFor(65))
        ));
        assert!(
            read_ps(
                "",
                "ps: unrecognized option: p\nBusyBox v1.37.0 multi-call binary."
            )
            .is_none(),
            "a usage message is not evidence that the process is gone"
        );
        assert!(read_ps("nonsense", "").is_none());
    }

    #[test]
    fn ps_elapsed_times_read_as_seconds() {
        assert_eq!(elapsed("05"), None);
        assert_eq!(elapsed("01:05"), Some(65));
        assert_eq!(elapsed("02:01:05"), Some(7_265));
        assert_eq!(elapsed("3-02:01:05"), Some(3 * 86_400 + 7_265));
        assert_eq!(elapsed("nonsense"), None);
    }
}
