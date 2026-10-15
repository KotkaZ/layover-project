//! `layover about`, `layover update`, and the Tower's daily look for a newer release.
//!
//! What a factory does about a release is its `[update]` table; the machinery is in
//! [`layover_update`]. This is the part a person sees.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use jiff::Timestamp;
use layover_core::config::Config;
use layover_update::{
    Available, LastCheck, Mode, Settings, Status, UpdateConfig, Version, check, compatibility, due,
    state, status,
};

use crate::commands::Failure;

/// How often the Tower's watcher wakes to see whether a check is due.
const WAKE: Duration = Duration::from_mins(1);

/// What this binary is: its version, the platform it was built for, the layout of the state
/// directory it reads and writes, the HTTP API it serves, and the oldest release it can be
/// updated from in one step. `--json` is for a program — the updater asks a downloaded binary
/// this before it trusts it.
#[must_use]
pub fn about(json: bool) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let layout = layover_store::layout::CURRENT;
    if json {
        let mut out = serde_json::json!({
            "version": version,
            "target": layover_update::TARGET,
            "layout": layout,
            "api": layover_http::API_VERSION,
            "min_upgrade_from": layover_update::MIN_UPGRADE_FROM,
        })
        .to_string();
        out.push('\n');
        return out;
    }
    format!(
        "Layover {version} for {}\n  state layout {layout}, HTTP API {}\n  updates from {} or \
         later in one step\n",
        layover_update::TARGET,
        layover_http::API_VERSION,
        layover_update::MIN_UPGRADE_FROM,
    )
}

/// The factory's `[update]` table: its own when it loads, the defaults when there is no factory.
fn update_config(path: &Path) -> Result<UpdateConfig, Failure> {
    if !path.exists() {
        return Ok(UpdateConfig::default());
    }
    Config::load(path)
        .map(|config| config.update)
        .map_err(|error| error.to_string())
}

/// `layover update check`: looks now, whatever the schedule, and says what it found.
///
/// # Errors
///
/// Returns why nothing could be found: switched off, offline, or the factory does not load.
pub fn check_now(path: &Path) -> Result<String, Failure> {
    let config = update_config(path)?;
    let settings = Settings::from_env();
    let running = Version::running();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Layover {running} ({}), {} channel.",
        layover_update::TARGET,
        config.channel
    );

    check(&settings, config.channel, Timestamp::now()).map_err(|error| error.to_string())?;
    let status = status(&settings, &config, &running);
    if let Some(available) = &status.available {
        describe(available, &mut out);
    } else if let Some(newest) = newest(&status)
        && status.skipped.contains(&newest.version)
    {
        let _ = writeln!(
            out,
            "{} is the newest release, and you skipped it. `layover update skip --clear` offers \
             it again.",
            newest.version
        );
    } else {
        out.push_str("This is the newest release.\n");
    }
    if config.mode == Mode::Off {
        out.push_str(
            "This factory's `[update] mode` is \"off\": its Tower does not look by itself.\n",
        );
    }
    Ok(out)
}

fn newest(status: &Status) -> Option<&layover_update::Release> {
    status.last.as_ref()?.newest.as_ref()
}

/// Says what is available, and what an existing factory would notice.
fn describe(available: &Available, out: &mut String) {
    let release = &available.release;
    let when = release
        .published_at
        .map(|at| format!("released {}, ", at.strftime("%Y-%m-%d")))
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "{} is available — {when}{}{}.",
        release.version,
        available.kind,
        if release.prerelease {
            ", a pre-release"
        } else {
            ""
        }
    );
    if let Some(compatibility) = compatibility(&release.notes) {
        for line in compatibility.lines() {
            let _ = writeln!(out, "  {line}");
        }
    }
    let _ = writeln!(out, "Release notes: {}", release.page);
    out.push_str(
        "Install it the way you installed Layover: \
         https://kotkaz.github.io/layover-project/install.html\n",
    );
}

/// `layover update skip`: stops offering `version`, or with `clear`, every skipped version again.
///
/// # Errors
///
/// Returns why the version could not be read or the machine's state written.
pub fn skip(version: Option<&str>, clear: bool) -> Result<String, Failure> {
    let settings = Settings::from_env();
    if clear {
        state::clear_skipped(&settings.dir).map_err(|error| error.to_string())?;
        return Ok("No version is skipped now.\n".to_owned());
    }
    let Some(version) = version else {
        return Err("name the version to skip, or say --clear".to_owned());
    };
    let version: Version = version.parse().map_err(|error| format!("{error}"))?;
    state::skip(&settings.dir, &version).map_err(|error| error.to_string())?;
    Ok(format!(
        "{version} will not be offered on this machine. A newer release will be.\n"
    ))
}

/// Looks for a newer release on the factory's schedule while the process lives, and says once
/// when one is found. Nothing at all when the factory or the machine has checks off.
///
/// Every Tower on a machine shares what was last found, so a second factory starting later does
/// not ask again.
pub fn watch(config: UpdateConfig, announce: impl Fn(String) + Send + 'static) {
    let settings = Settings::from_env();
    if config.mode == Mode::Off || settings.switched_off {
        return;
    }
    let watching = std::thread::Builder::new()
        .name("layover-update".to_owned())
        .spawn(move || {
            let running = Version::running();
            let mut said: Option<Version> = None;
            loop {
                let now = Timestamp::now();
                let last = state::last_check(&settings.dir, config.channel);
                if due(last.as_ref(), config.check_every(), now) {
                    // A failure is recorded and retried; it never stops the Tower.
                    let _ = check(&settings, config.channel, now);
                }
                if let Some(available) = status(&settings, &config, &running).available
                    && said.as_ref() != Some(&available.release.version)
                {
                    announce(format!(
                        "Layover {} is available ({}). `layover update check` says what changes.",
                        available.release.version, available.kind
                    ));
                    said = Some(available.release.version);
                }
                std::thread::sleep(WAKE);
            }
        });
    if let Err(error) = watching {
        eprintln!("Could not start looking for updates: {error}");
    }
}

/// One line for `doctor`: what runs, and what this machine last learned about releases.
#[must_use]
pub fn doctor_line(config: &UpdateConfig, now: Timestamp) -> String {
    let settings = Settings::from_env();
    let status = status(&settings, config, &Version::running());
    let mut line = format!("Layover {}", status.running);
    if status.switched_off {
        line.push_str("; update checks are off on this machine (LAYOVER_UPDATE=off).");
        return line;
    }
    match (&status.available, &status.last) {
        (Some(available), _) => {
            let _ = write!(
                line,
                "; {} is available ({}): `layover update check` says what changes.",
                available.release.version, available.kind
            );
        }
        (None, Some(last)) if last.succeeded_at.is_some() => {
            line.push_str(", the newest release");
            if let Some(newest) = &last.newest
                && status.skipped.contains(&newest.version)
            {
                let _ = write!(line, " but {}, which was skipped", newest.version);
            }
            line.push('.');
        }
        _ => {}
    }
    let _ = write!(line, " {}", last_checked(status.last.as_ref(), now));
    if config.mode == Mode::Off {
        line.push_str(" This factory does not look by itself (`[update] mode = \"off\"`).");
    }
    line
}

/// When this machine last looked, and whether it worked.
fn last_checked(last: Option<&LastCheck>, now: Timestamp) -> String {
    let Some(last) = last else {
        return "No check for updates has run on this machine yet.".to_owned();
    };
    let ago = |at: Timestamp| {
        let hours = now.duration_since(at).as_hours().max(0);
        if hours < 48 {
            format!("{hours} hour(s) ago")
        } else {
            format!("{} day(s) ago", hours / 24)
        }
    };
    match (&last.error, last.succeeded_at) {
        (None, Some(at)) => format!("Checked {}.", ago(at)),
        (Some(error), Some(at)) => format!(
            "The last {} check(s) failed ({error}); the last that worked was {}.",
            last.failures,
            ago(at)
        ),
        (Some(error), None) => format!("No check has worked yet: {error}."),
        (None, None) => format!("Checked {}.", ago(last.attempted_at)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layover_update::Channel;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("valid")
    }

    #[test]
    fn about_says_what_an_updater_asks() {
        let said: serde_json::Value = serde_json::from_str(&about(true)).expect("JSON");
        assert_eq!(said["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(said["target"], layover_update::TARGET);
        assert_eq!(said["layout"], layover_store::layout::CURRENT);
        assert_eq!(said["api"], env!("CARGO_PKG_VERSION"), "released together");
        assert_eq!(said["min_upgrade_from"], "1.0.0");
        assert!(about(false).starts_with("Layover "));
    }

    #[test]
    fn a_check_that_keeps_failing_says_when_one_last_worked() {
        let now = at("2026-10-18T09:00:00Z");
        let mut last = LastCheck {
            channel: Channel::Stable,
            attempted_at: at("2026-10-18T08:00:00Z"),
            succeeded_at: Some(at("2026-10-15T09:00:00Z")),
            error: Some("could not reach github.com".to_owned()),
            failures: 4,
            newest: None,
            etag: None,
        };
        let said = last_checked(Some(&last), now);
        assert!(said.contains("The last 4 check(s) failed"), "{said}");
        assert!(said.contains("3 day(s) ago"), "{said}");

        last.error = None;
        last.succeeded_at = Some(at("2026-10-18T06:00:00Z"));
        assert_eq!(last_checked(Some(&last), now), "Checked 3 hour(s) ago.");
        assert!(last_checked(None, now).contains("No check"));
    }
}
