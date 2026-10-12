//! `[layover]` keys that are accepted and would otherwise do nothing without saying so.

use crate::config::Config;

use super::Diagnostic;

/// `state_dir` is not honoured: everything Layover keeps is in `.layover/` beside the factory
/// definition, whatever it says. Accepted, because refusing it would stop factories that have
/// always loaded; warned about, because a key that is silently ignored is a lie in the file.
pub(super) fn check_state_dir_is_not_set(config: &Config, found: &mut Vec<Diagnostic>) {
    if let Some(dir) = &config.layover.state_dir {
        found.push(Diagnostic::warning(format!(
            "`[layover] state_dir = \"{}\"` is not honoured: history, the journal, Hangars and \
             live-run records are always in `.layover/` beside this file. Remove it",
            dir.display()
        )));
    }
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, parse, warnings};

    #[test]
    fn a_state_dir_that_would_be_ignored_is_a_warning() {
        let config = parse("[layover]\nstate_dir = \"/var/layover\"\n");

        assert_mentions(&warnings(&config), "`[layover] state_dir");
    }

    #[test]
    fn a_factory_that_does_not_set_one_hears_nothing_about_it() {
        let config = parse("[layover]\nwork_dir = \"work\"\n");

        assert!(
            warnings(&config)
                .iter()
                .all(|warning| !warning.contains("state_dir")),
            "{:?}",
            warnings(&config)
        );
    }
}
