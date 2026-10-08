//! Checks that what agents run on can say what it cost.
//!
//! A runner started without the output its CLI prints a cost in records every run as reporting
//! nothing. Nothing fails: the work is done, the spend reads as unknown on the dashboard, and Fuel
//! and the Reserve — which move only on measured figures — never bind it. That is exactly the
//! failure an unattended factory hides best, so it is said before the first run.
//!
//! Only what a change to the command line would fix is warned about. Codex prints no price however
//! it is run, and a warning nobody could ever clear is one people learn to skip; so Codex is
//! flagged only where a rate card is waiting for the token counts `--json` would give it.

use std::collections::BTreeMap;

use crate::agent::AgentName;
use crate::config::Config;
use crate::cost::reporting::{Cli, Reporting};
use crate::model::ModelChoice;

use super::Diagnostic;

pub(super) fn check_runners_report_cost(config: &Config, found: &mut Vec<Diagnostic>) {
    // Only runners some agent runs on: an unused one costs nothing.
    let mut users: BTreeMap<&str, Vec<&AgentName>> = BTreeMap::new();
    for (name, agent) in &config.agents {
        if let Some(runner) = agent.runner.as_ref().or(config.defaults.runner.as_ref()) {
            users.entry(runner.as_str()).or_default().push(name);
        }
    }

    for (runner_name, agents) in users {
        // An unknown runner is reported on its own.
        let Some(runner) = config.runners.get(runner_name) else {
            continue;
        };

        match Reporting::of(&runner.template()) {
            Reporting::Nothing {
                cli: Cli::Codex,
                add,
            } => {
                let priced: Vec<&AgentName> = agents
                    .into_iter()
                    .filter(|agent| {
                        ModelChoice::of(config, agent)
                            .model
                            .is_some_and(|model| config.rates.rates_for(&model).is_some())
                    })
                    .collect();
                if !priced.is_empty() {
                    found.push(Diagnostic::warning(format!(
                        "runner `{runner_name}` runs Codex without `{add}`, which is what prints \
                         its token counts, so the rate card can never estimate what {} cost; add \
                         `{add}` to its `command`",
                        listed(&priced)
                    )));
                }
            }
            Reporting::Nothing { cli, add } => {
                found.push(Diagnostic::warning(format!(
                    "runner `{runner_name}` runs {} without `{add}`, so every run of {} will \
                     report no cost: its spend reads as not reported, and Fuel and the Reserve \
                     cannot bind it. Add `{add}` to its `command`",
                    cli.name(),
                    listed(&agents)
                )));
            }
            Reporting::Dollars | Reporting::Credits | Reporting::Tokens | Reporting::Unknown => {}
        }
    }
}

/// `analyst`, `analyst and coder`, or `analyst, coder and reviewer`.
fn listed(agents: &[&AgentName]) -> String {
    let names: Vec<String> = agents.iter().map(|agent| format!("`{agent}`")).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::validate::testing::warnings;

    fn factory(runners: &str, extra: &str) -> Config {
        Config::from_toml(
            &format!(
                r#"
                [defaults]
                runner = "main"

                {runners}

                [agents.analyst]
                prompt = "analyse"
                entry = true
                model = "gpt-x"

                {extra}
                "#
            ),
            "costs.toml",
        )
        .expect("config parses")
    }

    fn cost_warnings(config: &Config) -> Vec<String> {
        warnings(config)
            .into_iter()
            .filter(|message| message.contains("cost"))
            .collect()
    }

    #[test]
    fn copilot_or_claude_without_their_json_output_is_warned_about_with_the_fix() {
        for (command, add) in [
            (
                r#"["copilot", "--allow-all-tools"]"#,
                "--output-format json",
            ),
            (r#"["claude", "-p"]"#, "--output-format stream-json"),
            (
                r#"["claude", "-p", "--output-format", "text"]"#,
                "--output-format stream-json",
            ),
        ] {
            let config = factory(&format!("[runners.main]\ncommand = {command}"), "");
            let found = cost_warnings(&config);
            assert_eq!(found.len(), 1, "{command}: {found:?}");
            assert!(found[0].contains(&format!("`{add}`")), "{found:?}");
            assert!(found[0].contains("`analyst`"), "names who it affects");
        }
    }

    #[test]
    fn a_runner_that_prints_its_cost_or_names_no_known_cli_is_left_alone() {
        for command in [
            r#"["copilot", "--allow-all-tools", "--output-format", "json"]"#,
            r#"["claude", "-p", "--output-format=stream-json"]"#,
            r#"["codex", "exec", "--json", "-"]"#,
            r#"["python", "stand-in.py"]"#,
        ] {
            let config = factory(&format!("[runners.main]\ncommand = {command}"), "");
            assert_eq!(cost_warnings(&config), Vec::<String>::new(), "{command}");
        }
    }

    #[test]
    fn a_runner_nobody_uses_is_not_warned_about() {
        let config = factory(
            r#"
            [runners.main]
            command = ["copilot", "--output-format", "json"]

            [runners.spare]
            command = ["claude", "-p"]
            "#,
            "",
        );
        assert_eq!(cost_warnings(&config), Vec::<String>::new());
    }

    #[test]
    fn codex_is_warned_about_only_where_a_rate_card_waits_for_its_token_counts() {
        let codex = r#"[runners.main]
            command = ["codex", "exec", "--model", "{model}", "-"]"#;

        assert_eq!(
            cost_warnings(&factory(codex, "")),
            Vec::<String>::new(),
            "no flag makes Codex print a price, so there is nothing to tell anybody to change"
        );

        let found = cost_warnings(&factory(
            codex,
            "[rates.gpt-x]\ninput_usd = 1.25\noutput_usd = 10.0",
        ));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("`--json`") && found[0].contains("rate card"));
    }
}
