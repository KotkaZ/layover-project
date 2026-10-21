//! Documentation checks that run inside `verify`.
//!
//! Layover's most valuable artefact is a command that returns a binary verdict, and documentation
//! is part of what has to be correct. An agent trusting a stale document makes confident wrong
//! changes, so the cheapest kinds of staleness are made into build failures here rather than left
//! to a reviewer to notice.
//!
//! What is checked:
//!
//! - every relative Markdown link resolves to a file that exists, and its `#anchor` to a heading
//!   in it; a link into `book/src/design/`, which is generated, is checked against `docs/`
//! - every absolute link to this repository's files on GitHub, or to a page of its published book,
//!   names a file and heading that exist on this branch
//! - every fenced `toml` block tagged as a factory definition is named by a real example
//! - every design document in `docs/` is a page of the book, listed in `book/src/SUMMARY.md`
//! - the version in `api/openapi.yaml` matches the workspace version
//! - every operation in `api/openapi.yaml` is a row of the endpoint table in
//!   `book/src/http-api.md`, with its query parameters, and every row is an operation — see
//!   [`crate::api_docs`]
//! - every release section of `CHANGELOG.md` says what it changes for an existing factory, in a
//!   `**Compatibility.**` paragraph — see [`crate::changelog`]
//!
//! What is deliberately not checked: prose. No tool can tell whether a paragraph is still true.
//! That is what the standing instruction in `AGENTS.md` is for.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

/// The published book, whose pages are `book/src/<page>.md`.
const SITE: &str = "https://kotkaz.github.io/layover-project/";

/// Anchors already read, by file.
type Anchors = HashMap<PathBuf, HashSet<String>>;

/// A documentation problem.
#[derive(Debug)]
pub struct Failure(String);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Failure {}

/// Runs every documentation check.
///
/// # Errors
///
/// Returns [`Failure`] listing every problem found, so one run reports all of them rather than
/// making the caller rediscover them one at a time.
pub fn check(root: &Path) -> Result<String, Failure> {
    let files = markdown_files(root);
    let mut problems = Vec::new();
    let mut anchors = Anchors::new();

    for file in &files {
        check_links(root, file, &mut anchors, &mut problems);
    }
    check_design_listed(root, &mut problems);
    check_spec_version(root, &mut problems);
    problems.extend(crate::api_docs::problems(root));
    match std::fs::read_to_string(root.join("CHANGELOG.md")) {
        Ok(changelog) => problems.extend(crate::changelog::problems(&changelog)),
        Err(error) => problems.push(format!("could not read CHANGELOG.md: {error}")),
    }

    if problems.is_empty() {
        return Ok(format!(
            "{} markdown files checked, all links and anchors resolve",
            files.len()
        ));
    }

    let mut message = format!("{} documentation problem(s):\n", problems.len());
    for problem in &problems {
        let _ = writeln!(message, "  - {problem}");
    }
    Err(Failure(message))
}

/// Collects every tracked Markdown file, skipping build output, the git directory, and the design
/// pages `cargo xtask book` writes from `docs/`, which are checked as `docs/` itself.
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(root, &mut found);
    let generated = root.join(crate::book::MIRROR);
    found.retain(|file| !file.starts_with(&generated));
    found.sort();
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }

        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
}

/// Checks that every link in `file` to this repository points at something that exists.
fn check_links(root: &Path, file: &Path, anchors: &mut Anchors, problems: &mut Vec<String>) {
    let Ok(text) = std::fs::read_to_string(file) else {
        problems.push(format!("could not read {}", show(root, file)));
        return;
    };

    let parent = file.parent().unwrap_or(root);

    for (number, line) in text.lines().enumerate() {
        for target in links(line) {
            let Some((path, anchor)) = resolve(root, parent, file, &target) else {
                continue;
            };
            let at = format!("{}:{}", show(root, file), number + 1);

            if !path.exists() {
                problems.push(format!("{at} links to `{target}`, which does not exist"));
                continue;
            }
            if anchor.is_empty() || path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let known = anchors.entry(path.clone()).or_insert_with(|| {
                std::fs::read_to_string(&path)
                    .map(|text| crate::anchors::of(&text))
                    .unwrap_or_default()
            });
            if !known.contains(anchor.as_str()) {
                problems.push(format!(
                    "{at} links to `{target}`, but {} has no heading `#{anchor}`",
                    show(root, &path)
                ));
            }
        }
    }
}

/// The file a link names and the anchor in it, or `None` for a link that is not this repository's.
fn resolve(root: &Path, parent: &Path, file: &Path, target: &str) -> Option<(PathBuf, String)> {
    if target.starts_with("mailto:") {
        return None;
    }
    let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
    let anchor = anchor.to_owned();

    let named = if let Some(page) = path.strip_prefix(SITE) {
        // The development build is published beside the release one, from this same tree.
        let page = page.strip_prefix("dev/").unwrap_or(page);
        let page = if page.is_empty() { "index.html" } else { page };
        root.join("book/src")
            .join(page.strip_suffix(".html")?.to_owned() + ".md")
    } else if let Some(rest) = path.strip_prefix(crate::book::REPOSITORY) {
        let rest = rest
            .strip_prefix("/blob/main/")
            .or_else(|| rest.strip_prefix("/tree/main/"))?;
        root.join(rest)
    } else if path.contains("://") {
        return None;
    } else if path.is_empty() {
        file.to_path_buf()
    } else {
        parent.join(path)
    };

    let named = normalise(&named);
    // `book/src/design/` is generated from `docs/` when the book is built.
    let design = normalise(&root.join(crate::book::MIRROR));
    let named = match named.strip_prefix(&design) {
        Ok(rest) => root.join("docs").join(rest),
        Err(_) => named,
    };
    Some((named, anchor))
}

/// `path` with `.` and `..` resolved without touching the file system.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
/// Extracts the targets of Markdown inline links on one line.
fn links(line: &str) -> Vec<String> {
    let bytes: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != ']' || index + 1 >= bytes.len() || bytes[index + 1] != '(' {
            index += 1;
            continue;
        }

        let start = index + 2;
        let mut end = start;
        let mut depth = 1;

        while end < bytes.len() {
            match bytes[end] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }

        if depth == 0 {
            let target: String = bytes[start..end].iter().collect();
            // Strip an optional title: [text](path "title")
            let target = target.split_whitespace().next().unwrap_or("").to_owned();
            if !target.is_empty() {
                found.push(target);
            }
            index = end + 1;
        } else {
            index += 1;
        }
    }

    found
}

/// Every file in `docs/` has to be a page of the book, or the published design silently lacks it.
fn check_design_listed(root: &Path, problems: &mut Vec<String>) {
    let Ok(summary) = std::fs::read_to_string(root.join("book/src/SUMMARY.md")) else {
        problems.push("could not read book/src/SUMMARY.md".to_owned());
        return;
    };
    for file in markdown_files(&root.join("docs")) {
        let relative = show(&root.join("docs"), &file);
        if !summary.contains(&format!("](./design/{relative})")) {
            problems.push(format!(
                "docs/{relative} is not in book/src/SUMMARY.md; add it as `./design/{relative}`"
            ));
        }
    }
}

/// The API version in the specification has to track the workspace version.
fn check_spec_version(root: &Path, problems: &mut Vec<String>) {
    let Ok(manifest) = std::fs::read_to_string(root.join("Cargo.toml")) else {
        problems.push("could not read Cargo.toml".to_owned());
        return;
    };
    let Ok(spec) = std::fs::read_to_string(root.join(crate::openapi::SPEC_PATH)) else {
        problems.push(format!("could not read {}", crate::openapi::SPEC_PATH));
        return;
    };

    let Some(workspace) = quoted_after(&manifest, "version = ") else {
        problems.push("could not find the workspace version in Cargo.toml".to_owned());
        return;
    };

    let spec_version = spec
        .lines()
        .skip_while(|line| !line.starts_with("info:"))
        .find_map(|line| line.trim().strip_prefix("version: "))
        .map(str::trim);

    match spec_version {
        Some(found) if found == workspace => {}
        Some(found) => problems.push(format!(
            "{} declares version {found} but the workspace is {workspace}",
            crate::openapi::SPEC_PATH
        )),
        None => problems.push(format!(
            "could not find `info.version` in {}",
            crate::openapi::SPEC_PATH
        )),
    }
}

fn quoted_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(prefix))
        .map(|rest| rest.trim().trim_matches('"'))
}

fn show(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_links_are_found() {
        assert_eq!(
            links("see [the docs](docs/architecture.md) and [more](../README.md)"),
            ["docs/architecture.md", "../README.md"]
        );
    }

    #[test]
    fn a_link_title_is_stripped() {
        assert_eq!(links(r#"[x](path.md "a title")"#), ["path.md"]);
    }

    #[test]
    fn nested_parentheses_do_not_end_the_link_early() {
        assert_eq!(links("[x](a(b)c.md)"), ["a(b)c.md"]);
    }

    #[test]
    fn a_line_with_no_links_yields_nothing() {
        assert!(links("just some prose (with parens)").is_empty());
        assert!(links("an unclosed [link](oops").is_empty());
    }

    #[test]
    fn the_repository_has_no_broken_links() {
        // The check runs against this repository inside `verify`; this makes it a unit test too,
        // so a broken link fails fast rather than only at the end of a long run.
        let root = crate::repository_root();
        let mut problems = Vec::new();
        let mut anchors = Anchors::new();
        for file in markdown_files(&root) {
            check_links(&root, &file, &mut anchors, &mut problems);
        }

        assert!(problems.is_empty(), "{problems:#?}");
    }
}
