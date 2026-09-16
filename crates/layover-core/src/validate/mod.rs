//! Load-time validation of a factory definition.
//!
//! Everything here runs before the first flight, because an unattended factory that discovers a
//! typo three agents deep has already spent money to find out. Errors block startup; warnings
//! describe shapes that are legal but known to be hazardous.
//!
//! The checks are grouped by what they are about — agents, routes, reach, pipelines and prompts —
//! and each group lives in its own file so that any one of them fits comfortably in view.

mod agents;
mod pipelines;
mod prompts;
mod reach;
mod routes;
mod wiring;

use crate::config::Config;
use crate::prompt::PromptSource;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The factory must not start.
    Error,
    /// Legal, but likely to misbehave.
    Warning,
}

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Builds an error.
    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }

    /// Builds a warning.
    pub(crate) fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }

    /// Returns `true` if this finding should block startup.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Returns `true` if any finding blocks startup.
#[must_use]
pub fn has_errors(diagnostics: &[Diagnostic]) -> bool {
    diagnostics.iter().any(Diagnostic::is_error)
}

/// Checks everything about a factory definition that does not need prompt files.
///
/// Prompt composition is checked separately by [`validate_prompts`], because reading prompt files
/// needs a [`PromptSource`] and callers that only have the TOML text should still get the rest of
/// the findings.
#[must_use]
pub fn validate(config: &Config) -> Vec<Diagnostic> {
    let mut found = Vec::new();

    routes::check_routes_name_known_agents(config, &mut found);
    agents::check_runners_exist(config, &mut found);
    agents::check_prompts_are_unambiguous(config, &mut found);
    agents::check_agents_are_described(config, &mut found);
    reach::check_entry_points(config, &mut found);
    routes::check_joins_are_unambiguous(config, &mut found);
    routes::check_read_write_fan_out(config, &mut found);
    routes::check_spawns_do_not_join(config, &mut found);
    reach::check_every_agent_is_within_reach(config, &mut found);
    pipelines::check_pipelines(config, &mut found);
    wiring::check_mcp_and_workspaces(config, &mut found);

    found
}

/// Checks that every agent's prompt composes, and that it only names declared flags.
///
/// Kept separate from [`validate`] so that the pure-TOML checks stay usable without a filesystem.
/// Run both before starting a factory.
#[must_use]
pub fn validate_prompts(config: &Config, source: &dyn PromptSource) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    prompts::check_prompt_files(config, source, &mut found);
    found
}

#[cfg(test)]
pub(crate) mod testing {
    use super::{Severity, validate};
    use crate::config::Config;

    pub(crate) const RUNNER: &str = r#"
        [runners.claude]
        command = ["claude", "-p", "{prompt}"]
    "#;

    pub(crate) fn parse(body: &str) -> Config {
        Config::from_toml(&format!("{RUNNER}{body}"), "test.toml").expect("config parses")
    }

    pub(crate) fn messages(config: &Config, severity: Severity) -> Vec<String> {
        validate(config)
            .into_iter()
            .filter(|d| d.severity == severity)
            .map(|d| d.message)
            .collect()
    }

    pub(crate) fn errors(config: &Config) -> Vec<String> {
        messages(config, Severity::Error)
    }

    pub(crate) fn warnings(config: &Config) -> Vec<String> {
        messages(config, Severity::Warning)
    }

    pub(crate) fn assert_mentions(found: &[String], needle: &str) {
        assert!(
            found.iter().any(|message| message.contains(needle)),
            "expected a finding mentioning `{needle}`, got {found:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{assert_mentions, errors, parse};
    use super::{Severity, has_errors, validate};

    #[test]
    fn a_minimal_factory_is_clean() {
        let config = parse(
            r#"
            [agents.planner]
            runner = "claude"
            description = "Breaks a goal into tasks"
            prompt = "plan"
            entry = true

            [agents.coder]
            runner = "claude"
            description = "Implements a task"
            prompt = "code"

            [[routes]]
            from = "planner"
            to = "coder"
            "#,
        );

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_factory_with_no_agents_is_an_error() {
        let config = parse("");

        assert!(has_errors(&validate(&config)));
        assert_mentions(&errors(&config), "defines no agents");
    }

    #[test]
    fn severity_orders_errors_before_warnings() {
        assert!(Severity::Error < Severity::Warning);
    }
}
