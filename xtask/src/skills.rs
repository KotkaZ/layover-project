//! Every agent skill in `skills/` is one an agent host can find.
//!
//! `gh skill install`, and every agent that reads the [Agent Skills] format, finds a skill by its
//! `SKILL.md` frontmatter alone: a `name` that follows the specification's rules and is its
//! directory's name, and a `description` that says when to use it. A skill that breaks either is
//! refused nowhere — it is silently not offered — so the rules are checked here, where breaking
//! them fails the build.
//!
//! [Agent Skills]: https://agentskills.io/specification

use std::path::Path;

use serde::Deserialize;

/// Where skills live: the first place `gh skill install` looks in a repository.
const DIR: &str = "skills";

/// The longest `name` and `description` the specification allows, in characters.
const NAME_LIMIT: usize = 64;
const DESCRIPTION_LIMIT: usize = 1024;

/// The frontmatter fields every skill must have.
#[derive(Deserialize)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// Says what is wrong with each skill in `root`'s `skills/`.
pub fn problems(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join(DIR)) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .collect();
    dirs.sort_by_key(std::fs::DirEntry::file_name);

    let mut found = Vec::new();
    for entry in dirs {
        let dir = entry.file_name().to_string_lossy().into_owned();
        match std::fs::read_to_string(entry.path().join("SKILL.md")) {
            Ok(text) => found.extend(
                check(&dir, &text)
                    .into_iter()
                    .map(|problem| format!("{DIR}/{dir}/SKILL.md {problem}")),
            ),
            Err(_) => found.push(format!(
                "{DIR}/{dir} has no SKILL.md, so no agent host will find it"
            )),
        }
    }
    found
}

/// What is wrong with the `SKILL.md` of the skill in the directory named `dir`.
fn check(dir: &str, text: &str) -> Vec<String> {
    let Some(yaml) = frontmatter(text) else {
        return vec!["does not open with `---` frontmatter".to_owned()];
    };
    let parsed: Frontmatter = match serde_yaml_bw::from_str(yaml) {
        Ok(parsed) => parsed,
        Err(error) => return vec![format!("has frontmatter that is not YAML: {error}")],
    };

    let mut found = Vec::new();
    match parsed.name.as_deref() {
        None => found.push("has no `name`".to_owned()),
        Some(name) => {
            if name != dir {
                found.push(format!(
                    "is named `{name}`, which must be its directory's name, `{dir}`"
                ));
            }
            if let Some(why) = misnamed(name) {
                found.push(format!("is named `{name}`, which {why}"));
            }
        }
    }
    match parsed.description.as_deref().map(str::trim) {
        None | Some("") => found
            .push("has no `description`, which is how an agent knows when to use it".to_owned()),
        Some(description) if description.chars().count() > DESCRIPTION_LIMIT => {
            found.push(format!(
                "has a {}-character `description`; the limit is {DESCRIPTION_LIMIT}",
                description.chars().count()
            ));
        }
        Some(_) => {}
    }
    found
}

/// The YAML between the opening `---` and the next line that is `---`.
fn frontmatter(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let end = rest.find("\n---")?;
    Some(&rest[..end])
}

/// Why `name` breaks the specification's naming rules, if it does.
fn misnamed(name: &str) -> Option<String> {
    if name.is_empty() || name.chars().count() > NAME_LIMIT {
        return Some(format!("must be 1 to {NAME_LIMIT} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Some("may use only lowercase letters, digits and hyphens".to_owned());
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Some("may not start or end with a hyphen, or have two together".to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# A skill\n")
    }

    #[test]
    fn a_skill_named_for_its_directory_and_described_passes() {
        assert!(check("install-layover", &skill("install-layover", "Installs it.")).is_empty());
        assert!(
            check(
                "install-layover",
                "---\r\nname: install-layover\r\ndescription: >-\r\n  Folded\r\n  text.\r\n---\r\n"
            )
            .is_empty(),
            "CRLF line endings and a folded description are both YAML"
        );
    }

    #[test]
    fn a_name_that_breaks_the_rules_is_named() {
        let found = check(
            "install-layover",
            &skill("Install--Layover", "Installs it."),
        );
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("directory's name"), "{found:?}");
        assert!(found[1].contains("lowercase"), "{found:?}");
        assert_eq!(check("a--b", &skill("a--b", "Fine.")).len(), 1);
        assert_eq!(check("a-", &skill("a-", "Fine.")).len(), 1);
    }

    #[test]
    fn a_skill_with_no_frontmatter_or_no_description_is_named() {
        assert_eq!(
            check("x", "# A skill with nothing above it\n"),
            ["does not open with `---` frontmatter"]
        );
        let found = check("x", "---\nname: x\n---\n");
        assert!(found[0].contains("no `description`"), "{found:?}");
        let found = check("x", &skill("x", &"y".repeat(DESCRIPTION_LIMIT + 1)));
        assert!(found[0].contains("the limit is 1024"), "{found:?}");
    }

    #[test]
    fn this_repositorys_skills_pass() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert!(root.join(DIR).join("install-layover/SKILL.md").exists());
        assert_eq!(problems(&root), Vec::<String>::new());
    }
}
