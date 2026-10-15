//! Every release says what an existing factory will notice.
//!
//! A release's section of `CHANGELOG.md` becomes its release notes, and the updater shows its
//! `**Compatibility.**` paragraph — before a person installs it, and in the dashboard — as the
//! answer to "what does this change for my factory". A release without one would leave that answer
//! blank at the one moment it is read. Every release from 1.1.0, when the convention began, has one;
//! this keeps it so, and asks the unreleased section for one as soon as it says anything.

/// The first release that carried a Compatibility paragraph.
const SINCE: (u64, u64, u64) = (1, 1, 0);

/// The paragraph every release section must carry.
const MARK: &str = "**Compatibility.**";

/// Says which sections of `changelog` lack a Compatibility paragraph.
pub fn problems(changelog: &str) -> Vec<String> {
    let mut found = Vec::new();
    for section in changelog.split("\n## ").skip(1) {
        let heading = section.lines().next().unwrap_or_default();
        let body: String = section.lines().skip(1).collect::<Vec<_>>().join("\n");
        let Some(name) = heading
            .strip_prefix('[')
            .and_then(|rest| rest.split(']').next())
        else {
            continue;
        };
        let wanted = if name == "Unreleased" {
            !body.trim().is_empty()
        } else {
            version(name).is_some_and(|version| version >= SINCE)
        };
        if wanted && !body.contains(MARK) {
            found.push(format!(
                "CHANGELOG.md: [{name}] has no `{MARK}` paragraph, which the updater shows as what \
                 the release changes for an existing factory"
            ));
        }
    }
    found
}

fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_without_a_compatibility_paragraph_is_named() {
        let changelog = "# Changelog\n\n## [Unreleased]\n\nNew things.\n\n\
                         ## [1.15.0] — 2026-10-17\n\nSummary.\n\n**Compatibility.** Additive.\n\n\
                         ## [1.14.0] — 2026-10-14\n\nSummary only.\n\n\
                         ## [1.0.0] — 2026-09-23\n\nBefore the convention.\n";
        let found = problems(changelog);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("[Unreleased]"));
        assert!(found[1].contains("[1.14.0]"));
    }

    #[test]
    fn an_empty_unreleased_section_asks_for_nothing() {
        assert!(problems("# Changelog\n\n## [Unreleased]\n\n## [1.0.0]\n\nOld.\n").is_empty());
    }

    #[test]
    fn this_repositorys_changelog_passes() {
        let changelog = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("CHANGELOG.md"),
        )
        .expect("reads");
        assert_eq!(problems(&changelog), Vec::<String>::new());
    }
}
