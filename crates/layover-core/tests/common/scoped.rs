//! A factory with pipeline-scoped routes, shaped like a real one, for the scope tests.

use std::fmt::Write as _;

use layover_core::Config;

/// Parses a factory body with a `claude` runner already declared.
pub fn config(body: &str) -> Config {
    Config::from_toml(
        &format!(
            r#"
            [runners.claude]
            command = ["claude", "-p", "--output-format", "stream-json"]

            {body}
            "#
        ),
        "scoped-test.toml",
    )
    .expect("the fixture parses")
}

/// Declares read-only agents named after themselves.
pub fn agents(names: &[&str]) -> String {
    let mut out = String::new();
    for name in names {
        let _ = writeln!(
            out,
            "[agents.{name}]\nrunner = \"claude\"\ndescription = \"{name}\"\nprompt = \"{name}\"\naccess = \"read-only\""
        );
    }
    out
}

/// Karl's factory, reduced to its route map: `devforge` and its follow-up share every route;
/// `eagle-eye` spawns a review per PR and consults two specialists; a monitor posts to Teams.
pub fn karls_factory() -> Config {
    config(&format!(
        r#"
        {}

        [agents.bob]
        runner = "claude"
        description = "builds"
        prompt = "bob"

        [reserve]
        fuel_usd = 200.0

        [pipelines.devforge]
        entry = "analyst"

        [pipelines.devforge-follow-up]
        entry = "azurix"
        trigger = {{ every = "2h" }}
        resumes = true

        [pipelines.eagle-eye]
        entry = "azurix"
        trigger = {{ every = "2h" }}

        [pipelines.chromium-webrtc-monitor]
        entry = "tars"
        trigger = {{ cron = "0 7 * * *" }}

        [[routes]]
        from = "analyst"
        to = ["sherlock", "golddigger", "bob"]
        pipelines = ["devforge", "devforge-follow-up"]
        [[routes]]
        from = ["sherlock", "golddigger"]
        to = ["analyst", "eagle"]
        pipelines = ["devforge", "devforge-follow-up"]
        [[routes]]
        from = "bob"
        to = ["wolf", "eagle", "analyst", "azurix"]
        pipelines = ["devforge", "devforge-follow-up"]
        [[routes]]
        from = "wolf"
        to = "bob"
        pipelines = ["devforge", "devforge-follow-up"]
        [[routes]]
        from = "eagle"
        to = ["bob", "sherlock", "golddigger"]
        pipelines = ["devforge", "devforge-follow-up"]
        [[routes]]
        from = "azurix"
        to = ["bob", "mailman"]
        pipelines = ["devforge", "devforge-follow-up"]

        [[routes]]
        from = "azurix"
        to = "eagle"
        mode = "spawn"
        pipelines = ["eagle-eye"]
        [[routes]]
        from = "eagle"
        to = ["azurix", "sherlock", "golddigger"]
        pipelines = ["eagle-eye"]
        [[routes]]
        from = ["sherlock", "golddigger"]
        to = "eagle"
        pipelines = ["eagle-eye"]

        [[routes]]
        from = "tars"
        to = "mailman"
        pipelines = ["chromium-webrtc-monitor"]
        "#,
        agents(&[
            "analyst",
            "sherlock",
            "golddigger",
            "wolf",
            "eagle",
            "azurix",
            "mailman",
            "tars"
        ])
    ))
}
