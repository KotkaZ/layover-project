//! `[copilot]`: what one Copilot AI credit costs this factory.

use layover_core::cost::USD_PER_COPILOT_CREDIT;
use layover_core::{Config, Severity, validate};

const BASE: &str = r#"
[layover]
work_dir = "work"

[defaults]
runner = "copilot"

[runners.copilot]
command = ["copilot", "--output-format", "json"]

[agents.analyst]
prompt = "analyse"
description = "analyses"
entry = true
"#;

fn parse(extra: &str) -> Config {
    Config::from_toml(&format!("{BASE}\n{extra}"), "copilot.toml").expect("parses")
}

fn errors(config: &Config) -> Vec<String> {
    validate(config)
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

#[test]
fn a_factory_that_says_nothing_prices_a_credit_at_the_published_cent() {
    let config = parse("");

    assert!((config.copilot.usd_per_credit - USD_PER_COPILOT_CREDIT).abs() < f64::EPSILON);
    assert!((USD_PER_COPILOT_CREDIT - 0.01).abs() < f64::EPSILON);
    assert!(errors(&config).is_empty(), "{:?}", errors(&config));
}

#[test]
fn a_factory_can_set_its_own_price_for_a_credit() {
    let config = parse("[copilot]\nusd_per_credit = 0.008\n");

    assert!((config.copilot.usd_per_credit - 0.008).abs() < f64::EPSILON);
    assert!(errors(&config).is_empty(), "{:?}", errors(&config));
}

#[test]
fn a_misspelt_key_is_refused_rather_than_ignored() {
    let text = format!("{BASE}\n[copilot]\nusd_per_credits = 0.02\n");

    assert!(Config::from_toml(&text, "copilot.toml").is_err());
}

#[test]
fn a_price_that_would_make_copilot_free_or_meaningless_is_an_error() {
    // Zero would record every Copilot run as free and measured, which switches Fuel and the
    // Reserve off for it while the configuration reads as though they bind.
    for bad in ["0", "0.0", "-0.01", "nan", "inf"] {
        let config = parse(&format!("[copilot]\nusd_per_credit = {bad}\n"));
        let found = errors(&config);

        assert!(
            found
                .iter()
                .any(|message| message.contains("[copilot] usd_per_credit")),
            "{bad}: {found:?}"
        );
    }
}
