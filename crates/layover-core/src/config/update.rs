//! `[update]`: whether this factory's Tower looks for a newer Layover, and what it does about one.
//!
//! Notifying is the default: the Tower asks once a day whether a newer release exists, and the
//! dashboard, `doctor` and `layover update check` say so. Nothing is downloaded and nothing
//! changes. See `docs/decisions.md` for why Layover updates itself at all.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How this factory's Tower treats new releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// Whether to look for new releases at all.
    #[serde(default)]
    pub mode: UpdateMode,
    /// Which releases to consider.
    #[serde(default)]
    pub channel: UpdateChannel,
    /// How often to look, in seconds. Written `check_every = "24h"`.
    #[serde(
        rename = "check_every",
        default = "default_check_every_sec",
        deserialize_with = "crate::duration::seconds"
    )]
    pub check_every_sec: u64,
}

/// Once a day: a release a day old is still new, and asking more often only spends requests.
fn default_check_every_sec() -> u64 {
    24 * 60 * 60
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            mode: UpdateMode::default(),
            channel: UpdateChannel::default(),
            check_every_sec: default_check_every_sec(),
        }
    }
}

impl UpdateConfig {
    /// How often to look for a new release.
    #[must_use]
    pub fn check_every(&self) -> Duration {
        Duration::from_secs(self.check_every_sec)
    }
}

/// Whether the Tower looks for new releases.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Never look. A person can still run `layover update check`.
    Off,
    /// Look, and say what was found. Download nothing, change nothing.
    #[default]
    Notify,
}

impl fmt::Display for UpdateMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Notify => "notify",
        })
    }
}

/// Which releases are considered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    /// Releases only.
    #[default]
    Stable,
    /// Pre-releases too, such as `1.15.0-rc.1`, for trying one before stable factories do.
    Prerelease,
}

impl fmt::Display for UpdateChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Prerelease => "prerelease",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn a_factory_that_says_nothing_is_told_about_stable_releases_daily() {
        let config = Config::from_toml("", "update.toml").expect("parses");

        assert_eq!(config.update.mode, UpdateMode::Notify);
        assert_eq!(config.update.channel, UpdateChannel::Stable);
        assert_eq!(config.update.check_every(), Duration::from_hours(24));
    }

    #[test]
    fn every_key_can_be_set() {
        let config = Config::from_toml(
            "[update]\nmode = \"off\"\nchannel = \"prerelease\"\ncheck_every = \"6h\"\n",
            "update.toml",
        )
        .expect("parses");

        assert_eq!(config.update.mode, UpdateMode::Off);
        assert_eq!(config.update.channel, UpdateChannel::Prerelease);
        assert_eq!(config.update.check_every(), Duration::from_hours(6));
    }

    #[test]
    fn an_unknown_key_or_mode_is_refused() {
        for text in [
            "[update]\nmode = \"sometimes\"\n",
            "[update]\nchanel = \"stable\"\n",
            "[update]\ncheck_every = \"daily\"\n",
        ] {
            assert!(Config::from_toml(text, "update.toml").is_err(), "{text}");
        }
    }
}
