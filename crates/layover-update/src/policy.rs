//! Whether `[update] mode = "auto"` may install a release with nobody watching.
//!
//! Each limit is one a person set or one that is never lifted:
//!
//! - a **major** release always waits for a person, and a minor one too under `allow = "patch"`;
//! - a release that changes the **state layout** waits for a person, because going back from it
//!   needs a snapshot nothing takes unattended;
//! - a release is not taken until it is **`min_age` old** — a day for a patch, two for a minor by
//!   default — the time in which a bad release can be withdrawn;
//! - it must be **staged**, **verified** (unless `require_provenance = false`), and **pass this
//!   factory's gate without a new warning**.
//!
//! *When* it begins — in the `window`, or once the factory has been idle for `idle_for` — is the
//! Tower's to decide; this says only whether it may.

use jiff::{SignedDuration, Timestamp};

use crate::check::Available;
use crate::gate::Gate;
use crate::provenance::Attestation;
use crate::stage::Staged;
use crate::version::Kind;
use crate::{UpdateAllow, UpdateConfig};

/// What an unattended install may do about a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Install it, at the next window or idle moment.
    Install,
    /// Not yet, but it may later as it is: cooling off, still staging, not yet gated.
    NotYet(String),
    /// Not unattended: it waits for a person, who may still install it by hand.
    Person(String),
}

/// What `update` lets an unattended install do about `available`, staged as `staged` and gated
/// as `gate`, at `now`.
#[must_use]
pub fn unattended(
    update: &UpdateConfig,
    available: &Available,
    staged: Option<&Staged>,
    gate: Option<&Gate>,
    now: Timestamp,
) -> Verdict {
    let version = &available.release.version;
    let min_age = match available.kind {
        Kind::Major => {
            return Verdict::Person(format!(
                "{version} is a major release, which waits for a person"
            ));
        }
        Kind::Minor if update.allow == UpdateAllow::Patch => {
            return Verdict::Person(format!(
                "{version} is a minor release, and `allow = \"patch\"`"
            ));
        }
        Kind::Minor => update.min_age.minor_sec,
        Kind::Patch => update.min_age.patch_sec,
    };
    let Some(published) = available.release.published_at else {
        return Verdict::Person(format!(
            "when {version} was released is not known, so its cool-off cannot be counted"
        ));
    };
    let ready_at = published
        .checked_add(SignedDuration::from_secs(
            i64::try_from(min_age).unwrap_or(i64::MAX),
        ))
        .unwrap_or(Timestamp::MAX);
    if now < ready_at {
        return Verdict::NotYet(format!(
            "{version} is cooling off until {}",
            ready_at.strftime("%Y-%m-%d %H:%M UTC")
        ));
    }
    let Some(staged) = staged.filter(|staged| staged.version == *version) else {
        return Verdict::NotYet(format!("{version} is not staged yet"));
    };
    if let Some(refused) = &staged.refused {
        return Verdict::NotYet(format!("staging {version} was refused: {refused}"));
    }
    if staged.crosses_layout {
        return Verdict::Person(format!(
            "{version} changes the state layout, which waits for a person"
        ));
    }
    let verified = matches!(staged.attestation, Some(Attestation::Verified { .. }));
    if update.require_provenance && !verified {
        return Verdict::NotYet(format!(
            "{version}'s provenance could not be verified; sign `gh` in on this machine"
        ));
    }
    if !staged.is_ready() {
        return Verdict::NotYet(format!("{version} is not ready to install yet"));
    }
    let Some(gate) = gate.filter(|gate| gate.version == *version) else {
        return Verdict::NotYet(format!(
            "{version} has not been checked against this factory"
        ));
    };
    if !gate.passes() {
        return Verdict::Person(format!("{version} refuses this factory"));
    }
    if !gate.passes_unattended() {
        return Verdict::Person(format!(
            "{version} gives warnings this version does not: {}",
            gate.new_warnings.join("; ")
        ));
    }
    Verdict::Install
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Release;
    use crate::version::Version;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("valid")
    }

    fn available(version: &str, kind: Kind) -> Available {
        Available {
            release: Release {
                version: version.parse().expect("valid"),
                tag: format!("v{version}"),
                prerelease: false,
                title: None,
                notes: String::new(),
                published_at: Some(at("2026-10-14T12:00:00Z")),
                page: String::new(),
            },
            kind,
        }
    }

    fn staged(version: &str) -> Staged {
        Staged {
            version: version.parse().expect("valid"),
            tag: format!("v{version}"),
            at: at("2026-10-14T13:00:00Z"),
            archive: "a.zip".to_owned(),
            sha256: Some("0".repeat(64)),
            attestation: Some(Attestation::Verified {
                signer: "release.yml".to_owned(),
            }),
            trusted_checksum_only: false,
            binary: Some("layover".into()),
            about: Some(crate::About::of_this_binary(1, "x")),
            crosses_layout: false,
            refused: None,
        }
    }

    fn gate(version: &str) -> Gate {
        Gate {
            version: version.parse::<Version>().expect("valid"),
            factory: "layover.toml".into(),
            at: at("2026-10-14T13:01:00Z"),
            not_run: None,
            errors: Vec::new(),
            new_warnings: Vec::new(),
        }
    }

    const LATER: &str = "2026-10-20T12:00:00Z";

    fn verdict(update: &UpdateConfig, version: &str, kind: Kind, now: &str) -> Verdict {
        unattended(
            update,
            &available(version, kind),
            Some(&staged(version)),
            Some(&gate(version)),
            at(now),
        )
    }

    #[test]
    fn patch_and_minor_are_installed_unattended_by_default_and_major_never() {
        let update = UpdateConfig::default();
        assert_eq!(
            verdict(&update, "1.14.1", Kind::Patch, LATER),
            Verdict::Install
        );
        assert_eq!(
            verdict(&update, "1.15.0", Kind::Minor, LATER),
            Verdict::Install
        );
        assert!(matches!(
            verdict(&update, "2.0.0", Kind::Major, LATER),
            Verdict::Person(why) if why.contains("major")
        ));
    }

    #[test]
    fn allow_patch_holds_a_minor_release_for_a_person() {
        let update = UpdateConfig {
            allow: UpdateAllow::Patch,
            ..UpdateConfig::default()
        };
        assert_eq!(
            verdict(&update, "1.14.1", Kind::Patch, LATER),
            Verdict::Install
        );
        assert!(matches!(
            verdict(&update, "1.15.0", Kind::Minor, LATER),
            Verdict::Person(why) if why.contains("allow = \"patch\"")
        ));
    }

    #[test]
    fn each_kind_cools_off_for_its_own_time() {
        let update = UpdateConfig::default();
        // Published 14 October 12:00: a patch is ready a day later, a minor two.
        assert!(matches!(
            verdict(&update, "1.14.1", Kind::Patch, "2026-10-15T11:59:00Z"),
            Verdict::NotYet(why) if why.contains("2026-10-15 12:00")
        ));
        assert_eq!(
            verdict(&update, "1.14.1", Kind::Patch, "2026-10-15T12:00:00Z"),
            Verdict::Install
        );
        assert!(matches!(
            verdict(&update, "1.15.0", Kind::Minor, "2026-10-15T12:00:00Z"),
            Verdict::NotYet(_)
        ));
        assert_eq!(
            verdict(&update, "1.15.0", Kind::Minor, "2026-10-16T12:00:00Z"),
            Verdict::Install
        );
    }

    #[test]
    fn a_layout_change_an_unverified_release_and_a_new_warning_each_wait() {
        let update = UpdateConfig::default();
        let found = available("1.15.0", Kind::Minor);
        let now = at(LATER);

        let mut moved = staged("1.15.0");
        moved.crosses_layout = true;
        assert!(matches!(
            unattended(&update, &found, Some(&moved), Some(&gate("1.15.0")), now),
            Verdict::Person(why) if why.contains("layout")
        ));

        let mut unverified = staged("1.15.0");
        unverified.attestation = Some(Attestation::Unavailable {
            reason: "no gh".to_owned(),
        });
        unverified.trusted_checksum_only = true;
        assert!(matches!(
            unattended(&update, &found, Some(&unverified), Some(&gate("1.15.0")), now),
            Verdict::NotYet(why) if why.contains("provenance")
        ));
        let trusting = UpdateConfig {
            require_provenance: false,
            ..UpdateConfig::default()
        };
        assert_eq!(
            unattended(
                &trusting,
                &found,
                Some(&unverified),
                Some(&gate("1.15.0")),
                now
            ),
            Verdict::Install
        );

        let mut warned = gate("1.15.0");
        warned.new_warnings.push("something new".to_owned());
        assert!(matches!(
            unattended(&update, &found, Some(&staged("1.15.0")), Some(&warned), now),
            Verdict::Person(why) if why.contains("something new")
        ));
        assert!(matches!(
            unattended(&update, &found, None, None, now),
            Verdict::NotYet(why) if why.contains("not staged")
        ));
    }
}
