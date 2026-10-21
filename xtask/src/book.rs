//! The design documents, as pages of the book.
//!
//! `docs/` is where the design lives, and where agents read it. The published book should carry
//! it too, so a reader can search it and follow a link into it — but copying it into `book/src/`
//! would make two sources of truth. So `cargo xtask book` writes `book/src/design/` from `docs/`
//! just before the book is built, rewriting the links that only make sense in the repository: one
//! to a book page becomes a link within the book, and one to code becomes a link to the file on
//! GitHub at the commit being published. `book/src/design/` is never committed.

use std::path::Path;

/// Where the repository is published, for links to files the book does not carry.
pub const REPOSITORY: &str = "https://github.com/KotkaZ/layover-project";

/// Where the generated pages go, relative to the repository root.
pub const MIRROR: &str = "book/src/design";

/// Writes `book/src/design/` from `docs/` under `root`, linking code at `git_ref`.
///
/// # Errors
///
/// Returns a description of the first file that could not be read or written.
pub fn generate(root: &Path, git_ref: &str) -> Result<String, String> {
    let mirror = root.join(MIRROR);
    if mirror.exists() {
        std::fs::remove_dir_all(&mirror)
            .map_err(|error| format!("could not clear {MIRROR}: {error}"))?;
    }

    let mut documents = Vec::new();
    collect(&root.join("docs"), &mut documents);
    documents.sort();

    for file in &documents {
        let relative = file
            .strip_prefix(root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("could not read {relative}: {error}"))?;
        let page = rewrite(&relative, &text, git_ref, |path| root.join(path).is_dir());
        let out = mirror.join(relative.trim_start_matches("docs/"));
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&out, page)
            .map_err(|error| format!("could not write {}: {error}", out.display()))?;
    }

    Ok(format!(
        "{} design page(s) written to {MIRROR}, linking code at {git_ref}",
        documents.len()
    ))
}

fn collect(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
}

/// `document` — a path such as `docs/decisions/cost.md` — as its page in the book.
pub fn rewrite(document: &str, text: &str, git_ref: &str, is_dir: impl Fn(&str) -> bool) -> String {
    let from = parent(document);
    let page = format!("{MIRROR}/{}", document.trim_start_matches("docs/"));
    let at = parent(&page);

    let mut out = String::with_capacity(text.len());
    let mut fenced = false;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if fenced || line.trim_start().starts_with("```") {
            out.push_str(line);
            continue;
        }
        out.push_str(&map_links(line, |target| {
            retarget(target, &from, &at, git_ref, &is_dir)
        }));
    }
    out
}

/// Where a link written in a `docs/` file should point from its page in the book, if elsewhere.
fn retarget(
    target: &str,
    from: &str,
    at: &str,
    git_ref: &str,
    is_dir: &impl Fn(&str) -> bool,
) -> Option<String> {
    if target.starts_with('#') || target.contains("://") || target.starts_with("mailto:") {
        return None;
    }
    let (path, anchor) = target.split_once('#').unwrap_or((target, ""));
    if path.is_empty() {
        return None;
    }
    let resolved = normalise(&format!("{from}/{path}"))?;
    // The mirror has the same layout as `docs/`, so links between design pages stand as written.
    if resolved.starts_with("docs/") {
        return None;
    }
    let suffix = if anchor.is_empty() {
        String::new()
    } else {
        format!("#{anchor}")
    };
    if resolved.starts_with("book/src/") {
        return Some(format!("{}{suffix}", relative(at, &resolved)));
    }
    let kind = if is_dir(&resolved) { "tree" } else { "blob" };
    Some(format!("{REPOSITORY}/{kind}/{git_ref}/{resolved}{suffix}"))
}

/// Replaces the target of every inline link on `line` that `change` has a new one for.
pub fn map_links(line: &str, mut change: impl FnMut(&str) -> Option<String>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != ']' || chars.get(index + 1) != Some(&'(') {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut end = start;
        let mut depth = 1;
        while end < chars.len() {
            match chars[end] {
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
        if depth != 0 {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let inside: String = chars[start..end].iter().collect();
        let target_len = inside.find(char::is_whitespace).unwrap_or(inside.len());
        let (target, title) = inside.split_at(target_len);
        out.push_str("](");
        match change(target) {
            Some(new) => out.push_str(&new),
            None => out.push_str(target),
        }
        out.push_str(title);
        out.push(')');
        index = end + 1;
    }
    out
}

/// A `/`-separated path with `.` and `..` resolved, or `None` if it climbs above the root.
fn normalise(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The directory part of a `/`-separated path.
fn parent(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(String::new(), |(dir, _)| dir.to_owned())
}

/// The path from directory `from` to `to`, both relative to the repository root.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
    let to: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; from.len() - shared];
    parts.extend(&to[shared..]);
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(document: &str, text: &str) -> String {
        rewrite(document, text, "v9.9.9", |path| {
            path == "examples/news-digest"
        })
    }

    #[test]
    fn a_link_between_design_pages_stands() {
        assert_eq!(
            page(
                "docs/decisions/cost.md",
                "[log](../decisions.md#still-open)\n"
            ),
            "[log](../decisions.md#still-open)\n"
        );
    }

    #[test]
    fn a_link_to_a_book_page_stays_inside_the_book() {
        assert_eq!(
            page(
                "docs/architecture.md",
                "[c](../book/src/configuration.md#flags)\n"
            ),
            "[c](../configuration.md#flags)\n"
        );
        assert_eq!(
            page(
                "docs/decisions/runners.md",
                "[r](../../book/src/runners.md)\n"
            ),
            "[r](../../runners.md)\n"
        );
    }

    #[test]
    fn a_link_to_code_goes_to_github_at_the_published_ref() {
        assert_eq!(
            page(
                "docs/architecture.md",
                "[p](../examples/planner.toml \"t\")\n"
            ),
            "[p](https://github.com/KotkaZ/layover-project/blob/v9.9.9/examples/planner.toml \"t\")\n"
        );
        assert_eq!(
            page("docs/risks.md", "[d](../examples/news-digest/)\n"),
            "[d](https://github.com/KotkaZ/layover-project/tree/v9.9.9/examples/news-digest)\n"
        );
    }

    #[test]
    fn code_blocks_and_external_links_are_left_alone() {
        let text = "```md\n[x](../README.md)\n```\n[y](https://example.com/a)\n";
        assert_eq!(page("docs/routing.md", text), text);
    }
}
