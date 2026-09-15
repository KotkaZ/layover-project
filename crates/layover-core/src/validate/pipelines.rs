//! Checks about pipelines: their entry agents, their schedules and their flags.

use std::collections::BTreeMap;

use crate::config::Config;

use super::Diagnostic;

pub(super) fn check_pipelines(config: &Config, found: &mut Vec<Diagnostic>) {
    check_entries_exist(config, found);
    check_flag_names_are_usable(config, found);
    check_flag_declarations_agree(config, found);
    check_schedules_are_survivable(config, found);
    check_schedules_fit_the_reserve(config, found);
}

/// Warns when the Reserve cannot fund the scheduled work at all.
///
/// Per-itinerary Fuel cannot see this: each tick of a scheduled pipeline is a *new* itinerary with
/// a *full* Fuel budget, so every chain stays inside its rail while the total runs away. An hourly
/// pipeline at `fuel_usd = 20` permits `24 × 20 = $480` a day.
///
/// Deliberately **not** a warning when the Reserve is merely lower than that worst case — being
/// lower is the entire reason to have a cap, and hitting it is a graceful pause rather than a
/// failure. What is reported is the unambiguously broken shape: a Reserve too small to fund even
/// one run of the pipeline, which can therefore never complete.
fn check_schedules_fit_the_reserve(config: &Config, found: &mut Vec<Diagnostic>) {
    let scheduled: Vec<_> = config.scheduled_pipelines().collect();
    if scheduled.is_empty() {
        return;
    }

    if config.reserve.is_unlimited() {
        found.push(Diagnostic::warning(
            "this factory has a scheduled pipeline but no `[reserve] fuel_usd`; each tick mints a \
             fresh itinerary with a fresh Fuel budget, so nothing bounds total spend",
        ));
        return;
    }

    for (name, pipeline) in scheduled {
        let per_itinerary = config.fuel_for(&pipeline.entry);
        if per_itinerary > config.reserve.fuel_usd {
            found.push(Diagnostic::warning(format!(
                "pipeline `{name}` may spend ${per_itinerary:.2} on one itinerary but `[reserve] \
                 fuel_usd` is ${:.2}; it could never run to completion",
                config.reserve.fuel_usd
            )));
        }
    }
}

fn check_entries_exist(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, pipeline) in &config.pipelines {
        if !config.agents.contains_key(&pipeline.entry) {
            found.push(Diagnostic::error(format!(
                "pipeline `{name}` enters at unknown agent `{}`",
                pipeline.entry
            )));
        }
    }
}

/// A flag name appears inside `@include(...)`, so it has to be an identifier.
fn check_flag_names_are_usable(config: &Config, found: &mut Vec<Diagnostic>) {
    for (name, pipeline) in &config.pipelines {
        for flag in pipeline.flags.keys() {
            if !is_identifier(flag) {
                found.push(Diagnostic::error(format!(
                    "pipeline `{name}` declares flag `{flag}`, which is not a usable name; use \
                     letters, digits and underscores, starting with a letter or underscore"
                )));
            }
        }
    }
}

/// Two pipelines may declare the same flag, but not with different defaults.
///
/// Prompts are shared between pipelines, so the same `@include(run_e2e)` line is read by every
/// pipeline that reaches that agent. Disagreeing defaults make the prompt mean different things
/// depending on which trigger fired, which is exactly the kind of thing nobody notices until the
/// output is wrong.
fn check_flag_declarations_agree(config: &Config, found: &mut Vec<Diagnostic>) {
    let mut seen: BTreeMap<&str, (bool, &str)> = BTreeMap::new();

    for (name, pipeline) in &config.pipelines {
        for (flag, spec) in &pipeline.flags {
            match seen.get(flag.as_str()) {
                Some((default, first)) if *default != spec.default => {
                    found.push(Diagnostic::warning(format!(
                        "flag `{flag}` defaults to {} in pipeline `{first}` and {} in pipeline \
                         `{name}`; the same prompt condition will mean different things",
                        default, spec.default
                    )));
                }
                Some(_) => {}
                None => {
                    seen.insert(flag.as_str(), (spec.default, name.as_str()));
                }
            }
        }
    }
}

/// Warns when a schedule can fire again before the previous run could plausibly have finished.
///
/// Runs are reentrant, so a schedule that outruns its own work does not queue — it piles up
/// concurrent copies of the same itinerary, each spending real money.
fn check_schedules_are_survivable(config: &Config, found: &mut Vec<Diagnostic>) {
    let timeout = config.defaults.timeout_sec;

    for (name, pipeline) in config.scheduled_pipelines() {
        let Some(schedule) = pipeline.trigger.schedule() else {
            continue;
        };
        let Some(gap) = schedule.min_gap_secs() else {
            continue;
        };

        if gap < timeout {
            found.push(Diagnostic::warning(format!(
                "pipeline `{name}` fires at least every {gap}s but a single run may take \
                 {timeout}s; runs are reentrant, so slow runs will overlap rather than queue"
            )));
        }
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use crate::validate::testing::{assert_mentions, errors, parse, warnings};
    use crate::validate::validate;

    const AGENT: &str = r#"
        [agents.analyst]
        runner = "claude"
        description = "analyses"
        prompt = "analyse"
    "#;

    #[test]
    fn a_pipeline_entering_at_an_unknown_agent_is_an_error() {
        let config = parse(&format!(
            r#"
            {AGENT}

            [pipelines.development]
            entry = "ghost"
            "#
        ));

        assert_mentions(&errors(&config), "unknown agent `ghost`");
    }

    #[test]
    fn a_flag_name_that_is_not_an_identifier_is_an_error() {
        // It has to survive being written inside `@include(...)`.
        let config = parse(&format!(
            r#"
            {AGENT}

            [pipelines.development]
            entry = "analyst"

            [pipelines.development.flags]
            "run-e2e" = {{ default = false }}
            "#
        ));

        assert_mentions(&errors(&config), "not a usable name");
    }

    #[test]
    fn two_pipelines_disagreeing_on_a_flag_default_warns() {
        let config = parse(&format!(
            r#"
            {AGENT}

            [pipelines.development]
            entry = "analyst"

            [pipelines.development.flags]
            run_e2e = {{ default = false }}

            [pipelines.nightly]
            entry = "analyst"
            trigger = {{ every = "1d" }}

            [pipelines.nightly.flags]
            run_e2e = {{ default = true }}
            "#
        ));

        assert_mentions(&warnings(&config), "will mean different things");
    }

    #[test]
    fn two_pipelines_agreeing_on_a_flag_default_is_fine() {
        let config = parse(&format!(
            r#"
            {AGENT}

            [pipelines.development]
            entry = "analyst"

            [pipelines.development.flags]
            run_e2e = {{ default = true }}

            [pipelines.nightly]
            entry = "analyst"
            trigger = {{ every = "1d" }}

            [pipelines.nightly.flags]
            run_e2e = {{ default = true }}
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_schedule_faster_than_a_run_warns() {
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 1800

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ every = "5m" }}
            "#
        ));

        assert_mentions(&warnings(&config), "will overlap rather than queue");
    }

    #[test]
    fn a_schedule_slower_than_a_run_is_fine() {
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 900

            [reserve]
            fuel_usd = 200.0

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_cron_that_fires_every_minute_is_warned_about() {
        // `* * * * *` states its own frequency, so the overlap check can read it off the minute
        // field without evaluating the expression.
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 1800

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ cron = "* * * * *" }}
            "#
        ));

        assert_mentions(&warnings(&config), "will overlap rather than queue");
    }

    #[test]
    fn a_stepped_cron_is_read_off_the_minute_field() {
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 1800

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ cron = "*/5 * * * *" }}
            "#
        ));

        assert_mentions(&warnings(&config), "at least every 300s");
    }

    #[test]
    fn an_hourly_cron_is_not_warned_about() {
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 1800

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ cron = "0 * * * *" }}
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_cron_whose_frequency_is_not_obvious_is_left_alone() {
        // A list or a range could mean almost anything. Guessing would produce warnings that are
        // not true, and a validator that cries wolf is one people stop reading.
        let config = parse(&format!(
            r#"
            [defaults]
            timeout_sec = 1800

            [reserve]
            fuel_usd = 0.0

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ cron = "0,30 * * * *" }}
            "#
        ));

        assert_mentions(&warnings(&config), "nothing bounds total spend");
        assert!(
            !warnings(&config).iter().any(|m| m.contains("could spend")),
            "an indeterminate cron cannot be projected"
        );
    }

    #[test]
    fn a_reserve_too_small_for_one_itinerary_warns() {
        // Unambiguously broken: the pipeline could never run to completion even once.
        let config = parse(&format!(
            r#"
            [defaults]
            fuel_usd = 20.0

            [reserve]
            fuel_usd = 5.0

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            "#
        ));

        assert_mentions(&warnings(&config), "could never run to completion");
    }

    #[test]
    fn a_reserve_below_the_theoretical_worst_case_is_not_flagged() {
        // 24 hourly ticks at $20 is $480 of theoretical worst case against a $120 Reserve — and
        // that is fine. Capping below worst case is the entire reason a cap exists, and reaching
        // it pauses the factory rather than breaking it.
        let config = parse(&format!(
            r#"
            [defaults]
            fuel_usd = 20.0

            [reserve]
            fuel_usd = 120.0

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }

    #[test]
    fn a_scheduled_factory_with_no_reserve_warns() {
        let config = parse(&format!(
            r#"
            [reserve]
            fuel_usd = 0.0

            {AGENT}

            [pipelines.review-bot]
            entry = "analyst"
            trigger = {{ every = "1h" }}
            "#
        ));

        assert_mentions(&warnings(&config), "nothing bounds total spend");
    }

    #[test]
    fn a_manual_only_factory_needs_no_reserve() {
        // Nobody is spending money while nobody is pressing the button.
        let config = parse(&format!(
            r#"
            [reserve]
            fuel_usd = 0.0

            {AGENT}

            [pipelines.development]
            entry = "analyst"
            trigger = "manual"
            "#
        ));

        assert_eq!(validate(&config), Vec::new());
    }
}
