//! `[update]` settings that would make the Tower poll GitHub rather than check it.

use crate::config::{Config, UpdateMode};

use super::Diagnostic;

/// The shortest `check_every` accepted. A release is news for days; asking more often than hourly
/// spends requests, and a factory's share of a corporate NAT's, on nothing.
const SHORTEST: u64 = 60 * 60;

/// A `check_every` shorter than an hour is refused, unless the factory never checks.
pub(super) fn check_update(config: &Config, found: &mut Vec<Diagnostic>) {
    let update = &config.update;
    if update.mode != UpdateMode::Off && update.check_every_sec < SHORTEST {
        found.push(Diagnostic::error(format!(
            "`[update] check_every` is {}s: a release is news for days, so the Tower checks at \
             most hourly. Say \"1h\" or more, or leave it out for once a day",
            update.check_every_sec
        )));
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse};

    #[test]
    fn checking_more_often_than_hourly_is_refused() {
        let about_updates = |text: &str| {
            errors(&parse(text))
                .into_iter()
                .filter(|error| error.contains("[update]"))
                .collect::<Vec<_>>()
        };
        assert_mentions(
            &about_updates("[update]\ncheck_every = \"10m\"\n"),
            "`[update] check_every` is 600s",
        );
        assert!(about_updates("[update]\ncheck_every = \"1h\"\n").is_empty());
        assert!(
            about_updates("[update]\nmode = \"off\"\ncheck_every = \"10m\"\n").is_empty(),
            "a factory that never checks is not asked how often"
        );
    }
}
