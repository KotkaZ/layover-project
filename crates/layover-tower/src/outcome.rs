//! How a finished run is described in history: its outcome, its exit code, and why it ended.
//!
//! # Why the reason is read from the transcript
//!
//! A run that fails in one second has usually said why — a CLI that cannot read its MCP
//! configuration, a credential that was refused — and it said so on its own output. That output
//! goes to the Hangar's transcript, which nobody opens from a dashboard. Without a line of it in
//! the record, the list shows `failed` and nothing else, and the person looking has to go and find
//! a file on disk to learn what the run already told them.
//!
//! The line is picked, not summarised. Layover calls no model, so the best it can honestly do is
//! choose the line that most looks like the reason — the last one that says *error* or *failed* —
//! and fall back to the last thing the run printed. It is redacted and capped on the way in,
//! because a transcript is exactly where a credential that failed to authenticate gets printed.

use layover_core::run::Outcome;

use crate::wait::Ended;

/// Longest a failure reason may be, in bytes.
///
/// One line for a list, not a log. Anything longer is a transcript being pasted into a field that
/// is served over HTTP and shown in a table.
pub const MAX_REASON: usize = 300;

/// Words that mark a line as the likely reason a run failed.
const FAILURE_MARKERS: [&str; 8] = [
    "error",
    "failed",
    "fatal",
    "panic",
    "denied",
    "refused",
    "cannot",
    "not found",
];

/// How a run is recorded, given how it ended and what it said.
///
/// `halted` and `timed_out` are deliberately not `failed`: a rail stopping work is the system
/// doing its job, and colouring it like a crash teaches people to ignore the colour.
#[must_use]
pub const fn outcome_of(ended: Ended, succeeded: bool) -> Outcome {
    match ended {
        Ended::TimedOut => Outcome::TimedOut,
        Ended::Halted => Outcome::Halted,
        Ended::Exited if succeeded => Outcome::Succeeded,
        Ended::Exited => Outcome::Failed,
    }
}

/// A line explaining an outcome that is not self-evident.
///
/// A success speaks for itself. The supervisor's own interventions — a timeout, a Ground Stop —
/// say so, because a list that does not reads as though the agent chose to stop. A failure says
/// how the process exited and the line of its output most likely to be the reason.
#[must_use]
pub fn detail_for(ended: Ended, exit_code: Option<i32>, transcript: &str) -> Option<String> {
    match ended {
        Ended::TimedOut => {
            Some("the run outlived `timeout_sec` and its process tree was ended".to_owned())
        }
        Ended::Halted => Some("a Ground Stop was engaged and the run was ended".to_owned()),
        Ended::Exited if exit_code == Some(0) => None,
        Ended::Exited => {
            let how = exit_code.map_or_else(
                || "exited abnormally".to_owned(),
                |code| format!("exited with code {code}"),
            );

            Some(match failure_reason(transcript) {
                Some(reason) => format!("{how}: {reason}"),
                None => format!("{how}, and printed nothing"),
            })
        }
    }
}

/// The line of a transcript most likely to say why a run failed, redacted and capped.
///
/// The last line naming a failure wins over the last line outright, because a CLI that fails
/// often prints its error and then a line of cleanup or a usage hint after it.
#[must_use]
pub fn failure_reason(transcript: &str) -> Option<String> {
    let lines: Vec<&str> = transcript
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    let chosen = lines
        .iter()
        .rev()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            FAILURE_MARKERS.iter().any(|marker| lower.contains(marker))
        })
        .or_else(|| lines.last())?;

    let redacted = layover_core::help::redact::secrets(chosen);
    Some(cap(&redacted))
}

/// Cuts `text` to [`MAX_REASON`] bytes on a character boundary, saying that it did.
fn cap(text: &str) -> String {
    if text.len() <= MAX_REASON {
        return text.to_owned();
    }

    let mut end = MAX_REASON;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_line_is_chosen_over_whatever_was_printed_after_it() {
        let transcript = "starting\nError: Failed to read MCP config file \"x\\mcp.json\"\nbye\n";

        assert_eq!(
            failure_reason(transcript).as_deref(),
            Some("Error: Failed to read MCP config file \"x\\mcp.json\"")
        );
    }

    #[test]
    fn without_an_error_line_the_last_thing_printed_is_the_best_available() {
        assert_eq!(
            failure_reason("one\ntwo\n\n").as_deref(),
            Some("two"),
            "blank lines are not a reason"
        );
    }

    #[test]
    fn a_silent_run_has_no_reason_to_give() {
        assert_eq!(failure_reason("  \n\n"), None);
        assert_eq!(
            detail_for(Ended::Exited, Some(2), "").as_deref(),
            Some("exited with code 2, and printed nothing")
        );
    }

    #[test]
    fn a_credential_in_the_reason_is_masked() {
        // A transcript is where a CLI that failed to authenticate prints what it tried. The
        // record is served over HTTP and shown in a table, so the value must not ride along.
        let reason = failure_reason(
            "error: GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789 was refused",
        )
        .expect("a reason");

        assert!(!reason.contains("ghp_abcdef"), "{reason}");
        assert!(reason.contains("GITHUB_TOKEN"), "{reason}");
    }

    #[test]
    fn a_long_reason_is_cut_to_one_line_for_a_list() {
        let reason = failure_reason(&format!("error: {}", "é".repeat(MAX_REASON))).expect("some");

        assert!(
            reason.len() <= MAX_REASON + '…'.len_utf8(),
            "{}",
            reason.len()
        );
        assert!(reason.ends_with('…'));
    }

    #[test]
    fn success_needs_no_explanation_and_a_rail_always_gets_one() {
        assert_eq!(detail_for(Ended::Exited, Some(0), "error: noise"), None);
        assert!(detail_for(Ended::TimedOut, None, "").is_some());
        assert!(detail_for(Ended::Halted, None, "").is_some());
    }

    #[test]
    fn a_process_killed_without_a_code_says_so() {
        let detail = detail_for(Ended::Exited, None, "fatal: killed").expect("explained");

        assert!(detail.starts_with("exited abnormally"), "{detail}");
        assert!(detail.contains("fatal: killed"), "{detail}");
    }
}
