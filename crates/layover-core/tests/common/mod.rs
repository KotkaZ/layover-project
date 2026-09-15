//! Shared fixtures for the reference-factory tests.
//!
//! Both `workitem_factory.rs` and `workitem_factory_runtime.rs` load the same example, so the
//! paths and constructors live here rather than being copied.
//!
//! Each test binary compiles this module separately and uses only part of it, so unused items are
//! expected rather than a sign of dead code.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use layover_core::{Config, PromptDir};

/// Directory holding the reference factory.
pub fn factory_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/workitem-factory")
}

/// Loads the reference factory definition.
pub fn factory() -> Config {
    Config::load(factory_dir().join("layover.toml"))
        .expect("the example factory must remain loadable")
}

/// A prompt source rooted at the reference factory's prompt directory.
pub fn prompts() -> PromptDir {
    let config = factory();
    PromptDir::new(factory_dir().join(&config.layover.prompt_dir))
}

/// Number of test/review cycles the reference factory is sized for.
pub const REVIEW_CYCLES: u32 = 8;
