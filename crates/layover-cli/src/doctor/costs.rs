//! What `layover doctor` says about runs whose cost is a floor rather than a figure — and, for each
//! runner they ran on, why, and what to change.
//!
//! "7 of 9 runs reported no cost" is true and not enough: the person reading it wants to know which
//! agent, and what to type. Almost always it is a runner started without the output its CLI prints
//! a cost in, and the command line says so. When the command line is right, the runs ended before
//! printing one, or were recorded by a release that did not price them — which is worth saying,
//! because history is never repriced and those runs will keep reading this way.

use std::collections::BTreeMap;

use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::CostSource;
use layover_core::cost::reporting::{Cli, Reporting};
use layover_core::model::ModelChoice;
use layover_core::run::RunRecord;

use super::{Finding, Report, Severity};

/// The runs that ran on one runner and were not measured.
#[derive(Default)]
struct Silent<'a> {
    agents: Vec<&'a AgentName>,
    runs: usize,
    estimated: usize,
}

/// Runs that were not measured, with what to do about each runner they ran on.
pub(super) fn unreported(config: &Config, runs: &[RunRecord], report: &mut Report) {
    let silent: Vec<&RunRecord> = runs
        .iter()
        .filter(|record| !record.source.is_measured())
        .collect();
    if silent.is_empty() {
        return;
    }

    // A proportion rather than a count: one silent run in a thousand is a runner quirk, and half
    // of them is a factory whose spending nobody can actually see.
    let share = silent.len() * 100 / runs.len().max(1);

    let mut by_runner: BTreeMap<Option<&str>, Silent<'_>> = BTreeMap::new();
    for record in &silent {
        let runner = config.agents.get(&record.agent).and_then(|agent| {
            agent
                .runner
                .as_deref()
                .or(config.defaults.runner.as_deref())
        });
        let group = by_runner.entry(runner).or_default();
        if !group.agents.contains(&&record.agent) {
            group.agents.push(&record.agent);
        }
        group.runs += 1;
        if record.source == CostSource::RateCard {
            group.estimated += 1;
        }
    }

    let mut advice: Vec<String> = by_runner
        .iter()
        .map(|(runner, group)| why(config, *runner, group))
        .collect();
    advice.push(
        "Every total that includes these is a floor, not a figure. Fuel is the rail that depends \
         on runners reporting honestly; the run cap is the one that does not, and it is what is \
         actually holding."
            .to_owned(),
    );

    report.findings.push(Finding {
        severity: if share >= 25 {
            Severity::Warning
        } else {
            Severity::Note
        },
        summary: format!(
            "{} of {} run(s) reported no cost ({share}%)",
            silent.len(),
            runs.len()
        ),
        advice: advice.join("\n    "),
    });
}

/// Why the runs on one runner were not measured, and what to change.
fn why(config: &Config, runner: Option<&str>, group: &Silent<'_>) -> String {
    let who = format!("{} ({} run(s))", listed(&group.agents), group.runs);
    let Some((name, definition)) =
        runner.and_then(|name| config.runners.get(name).map(|runner| (name, runner)))
    else {
        return format!("{who}: no longer in this factory, so what they ran on cannot be read.");
    };

    match Reporting::of(&definition.command) {
        Reporting::Nothing {
            cli: Cli::Codex,
            add,
        } => format!(
            "{who}: runner `{name}` runs Codex without `{add}`, so it prints neither a price nor \
             the token counts a rate card could estimate. Add `{add}`, and a `[rates.<model>]` \
             row for the model it runs on."
        ),
        Reporting::Nothing { cli, add } => format!(
            "{who}: runner `{name}` runs {} without `{add}`, which is the only output that \
             carries its cost. Add `{add}` to its `command`.",
            cli.name()
        ),
        Reporting::Tokens if group.estimated == group.runs => format!(
            "{who}: runner `{name}` runs Codex, which prints token counts and no price; these \
             were estimated from your rate card, which Fuel and the Reserve do not count."
        ),
        Reporting::Tokens => {
            let unpriced: Vec<String> = group
                .agents
                .iter()
                .filter_map(|agent| ModelChoice::of(config, agent).model)
                .filter(|model| config.rates.rates_for(model).is_none())
                .map(|model| format!("`[rates.{model}]`"))
                .collect();
            if unpriced.is_empty() {
                format!(
                    "{who}: runner `{name}` runs Codex, which prints token counts and no price, \
                     and the runs name no model a rate card could be keyed by; give the runner \
                     `--model`."
                )
            } else {
                format!(
                    "{who}: runner `{name}` runs Codex, which prints token counts and no price. \
                     Add {} to estimate them.",
                    unpriced.join(" and ")
                )
            }
        }
        reporting @ (Reporting::Credits | Reporting::Dollars) => format!(
            "{who}: runner `{name}` prints its cost, so these runs ended before they printed it — \
             killed, timed out or interrupted — or were recorded before Layover read it{}. History \
             is not repriced.",
            if reporting == Reporting::Credits {
                " (Copilot runs were first priced in 1.4.0)"
            } else {
                ""
            }
        ),
        Reporting::Unknown => format!(
            "{who}: runner `{name}` is not a CLI Layover reads a cost from. Have it print a JSON \
             line carrying `total_cost_usd`, or a `usage` with token counts for a rate card to \
             estimate."
        ),
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
    use super::*;
    use jiff::Timestamp;
    use layover_core::run::Outcome;

    fn config(runners: &str) -> Config {
        toml::from_str(&format!(
            r#"
            [layover]
            work_dir = "work"

            [defaults]
            runner = "copilot"

            {runners}

            [agents.analyst]
            prompt = "analyse"
            entry = true

            [agents.reviewer]
            prompt = "review"
            runner = "codex"
            model = "gpt-x"
            "#
        ))
        .expect("parses")
    }

    fn run(agent: &str, source: CostSource) -> RunRecord {
        RunRecord::started(
            layover_core::RunId::generate(),
            layover_core::flight::ItineraryId::generate(),
            AgentName::new(agent),
            Timestamp::now(),
        )
        .finished(Outcome::Succeeded, Timestamp::now())
        .costing(0.0, source, layover_core::cost::TokenUsage::default())
    }

    fn advice(config: &Config, runs: &[RunRecord]) -> String {
        let mut report = Report::default();
        unreported(config, runs, &mut report);
        report
            .findings
            .first()
            .map(|finding| finding.advice.clone())
            .unwrap_or_default()
    }

    #[test]
    fn silent_costs_are_a_note_when_rare_and_a_warning_when_common() {
        // One silent run in a thousand is a runner quirk; half of them is a factory whose
        // spending nobody can see.
        let config = config(
            r#"[runners.copilot]
            command = ["copilot", "--output-format", "json"]
            [runners.codex]
            command = ["codex", "exec", "--json", "--model", "{model}", "-"]"#,
        );

        let mut rare = Report::default();
        let mut runs: Vec<_> = (0..20)
            .map(|_| run("analyst", CostSource::CopilotCredits))
            .collect();
        runs.push(run("analyst", CostSource::Unreported));
        unreported(&config, &runs, &mut rare);
        assert_eq!(rare.findings[0].severity, Severity::Note);

        let mut common = Report::default();
        let half: Vec<_> = (0..10)
            .map(|index| {
                run(
                    "analyst",
                    if index % 2 == 0 {
                        CostSource::Reported
                    } else {
                        CostSource::Unreported
                    },
                )
            })
            .collect();
        unreported(&config, &half, &mut common);
        assert_eq!(common.findings[0].severity, Severity::Warning);
    }

    #[test]
    fn a_runner_without_its_json_output_is_named_with_what_to_add() {
        let config = config(
            r#"[runners.copilot]
            command = ["copilot", "--allow-all-tools"]
            [runners.codex]
            command = ["codex", "exec", "--model", "{model}", "-"]"#,
        );
        let said = advice(
            &config,
            &[
                run("analyst", CostSource::Unreported),
                run("analyst", CostSource::Unreported),
                run("reviewer", CostSource::Unreported),
            ],
        );

        assert!(
            said.contains("`analyst` (2 run(s)): runner `copilot` runs Copilot CLI without `--output-format json`"),
            "{said}"
        );
        assert!(
            said.contains("runner `codex` runs Codex without `--json`")
                && said.contains("[rates.<model>]"),
            "{said}"
        );
    }

    #[test]
    fn a_runner_that_prints_its_cost_says_the_runs_ended_first_or_predate_pricing() {
        let config = config(
            r#"[runners.copilot]
            command = ["copilot", "--output-format", "json"]
            [runners.codex]
            command = ["codex", "exec", "--json", "--model", "{model}", "-"]"#,
        );
        let said = advice(&config, &[run("analyst", CostSource::Unreported)]);

        assert!(said.contains("ended before they printed it"), "{said}");
        assert!(
            said.contains("1.4.0") && said.contains("not repriced"),
            "{said}"
        );
    }

    #[test]
    fn codex_tokens_ask_for_a_rate_card_row_until_one_estimates_them() {
        let codex = r#"[runners.copilot]
            command = ["copilot", "--output-format", "json"]
            [runners.codex]
            command = ["codex", "exec", "--json", "--model", "{model}", "-"]"#;

        let said = advice(&config(codex), &[run("reviewer", CostSource::Unreported)]);
        assert!(said.contains("Add `[rates.gpt-x]`"), "{said}");

        let priced = config(&format!("{codex}\n[rates.gpt-x]\ninput_usd = 1.0"));
        let said = advice(&priced, &[run("reviewer", CostSource::RateCard)]);
        assert!(
            said.contains("estimated from your rate card") && said.contains("do not count"),
            "{said}"
        );
    }

    #[test]
    fn an_agent_no_longer_in_the_factory_is_said_to_be_gone() {
        let config = config(
            r#"[runners.copilot]
            command = ["copilot", "--output-format", "json"]
            [runners.codex]
            command = ["codex", "exec", "--json", "-"]"#,
        );
        let said = advice(&config, &[run("retired", CostSource::Unreported)]);

        assert!(
            said.contains("`retired` (1 run(s)): no longer in this factory"),
            "{said}"
        );
    }
}
