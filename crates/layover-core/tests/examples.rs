//! Guards against the documented configuration drifting away from the parser.
//!
//! `docs/architecture.md` shows a factory definition. If that shape stops loading, an agent
//! following the documentation would produce a configuration Layover rejects, and the
//! documentation would be actively misleading rather than merely stale.

use layover_core::{Config, validate};

#[test]
fn the_documented_example_parses_and_validates() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/planner.toml");

    let config = Config::load(path).expect("the documented example must remain loadable");
    let diagnostics = validate(&config);

    assert_eq!(
        diagnostics,
        Vec::new(),
        "the documented example must validate cleanly"
    );
    assert_eq!(config.agents.len(), 3);
    assert_eq!(config.runners.len(), 3);
    assert_eq!(config.entry_agents().count(), 1);
}
