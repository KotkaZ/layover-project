//! `[learnings]` numbers that would stop learnings working without saying so.

use crate::config::Config;

use super::Diagnostic;

/// A zero in `[learnings]` refuses every proposal, gives runs nothing, or makes "provisional" and
/// "permanent" mean nothing — and none of it fails anything a person would see. So it is refused.
///
/// `keep_lapsed_days` is the exception: zero there is a real choice, deleting a learning as soon as
/// it lapses.
pub(super) fn check_learning_rules(config: &Config, found: &mut Vec<Diagnostic>) {
    let learnings = &config.learnings;
    for (key, value, because) in [
        (
            "provisional_runs",
            learnings.provisional_runs,
            "a learning would lapse at the first run after it was proposed",
        ),
        (
            "confirm_after",
            learnings.confirm_after,
            "it reads as no rediscovery being needed, and one is; say 1 for the first",
        ),
        (
            "max_chars",
            learnings.max_chars,
            "every proposal would be refused as too long",
        ),
        (
            "max_per_run",
            learnings.max_per_run,
            "runs would be asked to propose learnings and never given any",
        ),
    ] {
        if value == 0 {
            found.push(Diagnostic::error(format!(
                "`[learnings] {key}` is 0: {because}. Leave it out for Layover's default"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse};

    #[test]
    fn a_zero_that_would_stop_learnings_working_is_refused() {
        for key in [
            "provisional_runs",
            "confirm_after",
            "max_chars",
            "max_per_run",
        ] {
            let config = parse(&format!("[learnings]\n{key} = 0\n"));
            assert_mentions(&errors(&config), &format!("`[learnings] {key}` is 0"));
        }
    }

    #[test]
    fn keeping_lapsed_learnings_for_no_days_is_a_choice() {
        let config = parse("[learnings]\nkeep_lapsed_days = 0\n");

        assert!(
            errors(&config)
                .iter()
                .all(|error| !error.contains("[learnings]")),
            "{:?}",
            errors(&config)
        );
    }
}
