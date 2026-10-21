//! The anchors a Markdown file's headings give it.
//!
//! A link to `page.md#some-heading` breaks silently when the heading is renamed or moved to
//! another page: the file still exists, so a file-level check passes, and the reader lands at the
//! top of the wrong page. This computes the anchors GitHub and mdBook both derive from a heading,
//! so the link check can say which ones no longer exist.

use std::collections::HashSet;

/// The anchor a heading's text becomes: lower-cased, spaces as hyphens, punctuation dropped.
///
/// GitHub and mdBook agree on this for everything the documentation uses: letters, digits,
/// hyphens and underscores are kept, whitespace becomes a hyphen, and the rest — backticks,
/// brackets, dashes, full stops, apostrophes — disappears. A link's text stands for the link.
pub fn slug(heading: &str) -> String {
    let text = link_text(heading);
    let mut out = String::with_capacity(text.len());
    for ch in text.trim().chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.extend(ch.to_lowercase());
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    out
}

/// Every anchor in `markdown`: one per heading outside a code block, with `-1`, `-2` … for a
/// repeated heading, plus any `id="…"` written in HTML.
pub fn of(markdown: &str) -> HashSet<String> {
    let mut found = HashSet::new();
    let mut counts = std::collections::HashMap::<String, usize>::new();
    let mut fenced = false;

    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(text) = heading(line) {
            let base = slug(text);
            let seen = counts.entry(base.clone()).or_insert(0);
            found.insert(if *seen == 0 {
                base.clone()
            } else {
                format!("{base}-{seen}")
            });
            *seen += 1;
        }
        let mut rest = line;
        while let Some(at) = rest.find("id=\"") {
            let after = &rest[at + 4..];
            if let Some(end) = after.find('"') {
                found.insert(after[..end].to_owned());
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    found
}

/// The text of an ATX heading, without its leading hashes or any closing ones.
fn heading(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim_end())
}

/// `[text](target)` as `text`, so a heading's link contributes only what it reads as.
fn link_text(heading: &str) -> String {
    let mut out = String::with_capacity(heading.len());
    let mut rest = heading;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|at| open + at) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|at| close + at) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn punctuation_goes_and_spaces_become_hyphens() {
        assert_eq!(
            slug("`[update]` — hearing about a newer Layover"),
            "update--hearing-about-a-newer-layover"
        );
        assert_eq!(
            slug("1. Resident + reentrant conflict"),
            "1-resident--reentrant-conflict"
        );
        assert_eq!(
            slug("A CLI home of the factory's own"),
            "a-cli-home-of-the-factorys-own"
        );
    }

    #[test]
    fn a_link_in_a_heading_counts_as_its_text() {
        assert_eq!(slug("See [the docs](x.md) now"), "see-the-docs-now");
    }

    #[test]
    fn a_repeated_heading_is_numbered_and_code_is_skipped() {
        let found = of("# Title\n\n## Why\n\n```sh\n# not a heading\n```\n\n## Why\n");
        assert!(found.contains("title"));
        assert!(found.contains("why"));
        assert!(found.contains("why-1"));
        assert!(!found.contains("not-a-heading"));
    }

    #[test]
    fn an_html_id_is_an_anchor() {
        assert!(of("<a id=\"here\"></a>\n").contains("here"));
    }
}
