//! Every length of time in `layover.toml` can be written as a duration — `"4h"` — under a name
//! without a unit, while the old names and bare numbers keep their meaning.

use layover_core::config::Config;

fn parse(text: &str) -> Result<Config, String> {
    Config::from_toml(text, "durations.toml").map_err(|error| format!("{error:?}"))
}

fn config(text: &str) -> Config {
    parse(text).unwrap_or_else(|error| panic!("{text} should parse: {error}"))
}

#[test]
fn a_timeout_can_be_written_as_a_duration_or_as_seconds() {
    assert_eq!(
        config("[defaults]\ntimeout = \"4h\"\n")
            .defaults
            .timeout_sec,
        14_400
    );
    assert_eq!(
        config("[defaults]\ntimeout_sec = 14400\n")
            .defaults
            .timeout_sec,
        14_400
    );
    assert_eq!(
        config("[defaults]\ntimeout_sec = \"90m\"\n")
            .defaults
            .timeout_sec,
        5_400
    );
}

#[test]
fn the_reserve_window_and_lapsed_learnings_take_whole_units() {
    let written = config("[reserve]\nwindow = \"2d\"\n\n[learnings]\nkeep_lapsed = \"3d\"\n");
    assert_eq!(written.reserve.window_hours, 48);
    assert_eq!(written.learnings.keep_lapsed_days, 3);

    let numbers = config("[reserve]\nwindow_hours = 36\n\n[learnings]\nkeep_lapsed_days = 9\n");
    assert_eq!(numbers.reserve.window_hours, 36);
    assert_eq!(numbers.learnings.keep_lapsed_days, 9);

    let error = parse("[reserve]\nwindow = \"90m\"\n").expect_err("not whole hours");
    assert!(error.contains("not a whole number of hours"), "{error}");
    let error = parse("[learnings]\nkeep_lapsed = \"36h\"\n").expect_err("not whole days");
    assert!(error.contains("not a whole number of days"), "{error}");
}

#[test]
fn a_join_timeout_can_be_written_as_a_duration() {
    let config = config(
        r#"
[agents.a]
prompt = "a"

[agents.b]
prompt = "b"

[[routes]]
from = "a"
to = "b"
timeout = "30m"
"#,
    );

    assert_eq!(config.routes[0].timeout_sec, Some(1_800));
}

#[test]
fn naming_a_value_twice_is_refused_rather_than_one_quietly_winning() {
    let error = parse("[defaults]\ntimeout = \"4h\"\ntimeout_sec = 60\n").expect_err("twice");
    assert!(error.contains("duplicate"), "{error}");
}

#[test]
fn something_that_is_not_a_duration_is_refused_with_what_one_looks_like() {
    for bad in ["\"4 hours\"", "\"4w\"", "-5", "\"\""] {
        let error = parse(&format!("[defaults]\ntimeout = {bad}\n")).expect_err(bad);
        assert!(
            error.contains("duration") || error.contains("negative"),
            "{bad}: {error}"
        );
    }
}
