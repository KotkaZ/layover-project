//! Every key `layover.toml` accepts is documented, in the part of the book about its table.
//!
//! The keys are not listed by hand. Every table is `deny_unknown_fields`, so serde refuses a key it
//! does not know by naming every key it does; each table is probed with one nobody would write and
//! the list is read from the refusal. A field added to a struct is therefore a key this test asks
//! the book about, without anyone remembering to tell it.
//!
//! What it cannot see: free-form maps have no fixed keys to list — an MCP server's `env`, the event
//! shapes under `provider_errors.failed` and `carried_on` — and values are not checked, only keys.

use layover_core::{Config, ConfigError};

/// The key every table is probed with.
const PROBE: &str = "zz_probe";

/// One table a factory can write, and where its keys are documented.
struct Table {
    /// The table's header in a factory, empty for the top level.
    header: &'static str,
    /// How the headings of the sections of `configuration.md` about it begin; none for the whole
    /// page. A section runs to the next heading of any level, so a table's keys have to be
    /// documented with the table and not in a subsection about something else.
    sections: &'static [&'static str],
    /// Pages of the book that section hands its detail to. A key on one of them is documented.
    pages: &'static [&'static str],
}

/// Every table a factory can write.
const TABLES: &[Table] = &[
    Table {
        header: "",
        sections: &[],
        pages: &[],
    },
    Table {
        header: "[layover]",
        sections: &["## `[layover]`"],
        pages: &[],
    },
    Table {
        header: "[defaults]",
        sections: &["## `[defaults]`"],
        pages: &[],
    },
    Table {
        header: "[reserve]",
        sections: &["## `[reserve]`"],
        pages: &[],
    },
    Table {
        header: "[rates.model]",
        sections: &["## `[rates]`"],
        pages: &["cost.md"],
    },
    Table {
        header: "[copilot]",
        sections: &["## `[copilot]`"],
        pages: &["cost.md"],
    },
    Table {
        header: "[learnings]",
        sections: &["## `[learnings]`"],
        pages: &["learning.md"],
    },
    Table {
        header: "[update]",
        sections: &["## `[update]`"],
        pages: &["updating.md"],
    },
    Table {
        header: "[update.min_age]",
        sections: &["## `[update]`"],
        pages: &["updating.md"],
    },
    Table {
        header: "[runners.runner]",
        sections: &["## `[runners.*]`"],
        pages: &["runners.md"],
    },
    Table {
        header: "[runners.runner.mcp]",
        sections: &["## `[runners.*]`"],
        pages: &["runners.md"],
    },
    Table {
        header: "[runners.runner.provider_errors]",
        sections: &["## `[runners.*]`"],
        pages: &["runners.md"],
    },
    Table {
        header: "[agents.agent]",
        sections: &["## `[agents.*]`"],
        pages: &[],
    },
    Table {
        header: "[agents.agent.mcp.server]",
        sections: &["### MCP servers"],
        pages: &[],
    },
    Table {
        header: "[pipelines.pipeline]",
        sections: &["## `[pipelines.*]`"],
        pages: &["pipelines.md"],
    },
    Table {
        header: "[pipelines.pipeline.trigger]",
        sections: &["## `[pipelines.*]`"],
        pages: &["pipelines.md"],
    },
    Table {
        header: "[pipelines.pipeline.flags.flag]",
        // `[flags]` is documented as the same shape.
        sections: &["## `[pipelines.*]`", "## `[flags]`"],
        pages: &["pipelines.md"],
    },
    Table {
        header: "[pipelines.pipeline.agents.agent]",
        sections: &["## `[pipelines.*]`"],
        pages: &["pipelines.md"],
    },
    Table {
        header: "[flags.flag]",
        sections: &["## `[flags]`"],
        pages: &["pipelines.md"],
    },
    Table {
        header: "[[routes]]",
        sections: &["## `[[routes]]`"],
        pages: &["routes.md"],
    },
];

/// The older names of lengths of time carry their unit. `configuration.md` documents them once, as
/// a rule, so one mentioned anywhere on that page is documented.
const UNITS: &[&str] = &["_sec", "_hours", "_days"];

fn page(name: &str) -> String {
    let path = format!("{}/../../book/src/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
}

/// The keys serde accepts in `table`, read from its refusal of one it does not.
fn keys(table: &Table) -> Result<Vec<String>, String> {
    let factory = format!("{}\n{PROBE} = 1\n", table.header);
    let refusal = match Config::from_toml(&factory, "probe.toml") {
        Ok(_) => return Err("accepted an unknown key".to_owned()),
        Err(ConfigError::Parse { source, .. }) => source.to_string(),
        Err(other) => return Err(other.to_string()),
    };
    let unknown = format!("unknown field `{PROBE}`, ");
    let Some(listed) = refusal
        .lines()
        .find_map(|line| line.split_once(unknown.as_str()).map(|(_, rest)| rest))
    else {
        return Err(format!("refused the probe for another reason: {refusal}"));
    };
    let keys = expected(listed);
    if keys.is_empty() {
        return Err(format!("named no keys it accepts: {refusal}"));
    }
    Ok(keys)
}

/// The names in serde's ``expected `a` ``, ``expected `a` or `b` `` or
/// ``expected one of `a`, `b` ``.
fn expected(listed: &str) -> Vec<String> {
    listed
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The lines of `text` outside fenced blocks, each with whether it is a heading.
fn prose(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut fenced = false;
    text.lines().filter_map(move |line| {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            return None;
        }
        (!fenced).then(|| (line, line.starts_with('#')))
    })
}

/// The prose of the section whose heading begins `heading`, up to the next heading of any level.
fn section(text: &str, heading: &str) -> Option<String> {
    let mut lines = prose(text).skip_while(|(line, _)| !line.starts_with(heading));
    let (first, _) = lines.next()?;
    let mut found = first.to_owned();
    for (line, _) in lines.take_while(|(_, is_heading)| !is_heading) {
        found.push('\n');
        found.push_str(line);
    }
    Some(found)
}

/// Whether `text` documents `key`, outside fenced blocks — an example is not a definition — as
/// `` `key` ``, as an assignment such as `` `key = "4h"` ``, or as part of a table header such as
/// `` `[agents.<name>.mcp.<server>]` ``.
fn documents(text: &str, key: &str) -> bool {
    prose(text)
        .flat_map(|(line, _)| line.split('`').skip(1).step_by(2))
        .any(|span| span == key || assigns(span, key) || names_table(span, key))
}

fn assigns(span: &str, key: &str) -> bool {
    span.match_indices(key).any(|(at, _)| {
        let before = span[..at].chars().next_back();
        let after = span[at + key.len()..].trim_start();
        before.is_none_or(|c| c == ' ' || c == '{' || c == ',') && after.starts_with('=')
    })
}

fn names_table(span: &str, key: &str) -> bool {
    span.strip_prefix('[')
        .and_then(|span| span.strip_suffix(']'))
        .is_some_and(|path| {
            path.trim_matches(['[', ']'])
                .split('.')
                .any(|part| part == key)
        })
}

/// Whether `key` is the older, unit-carrying name of another of `keys`.
fn is_older_name(key: &str, keys: &[String]) -> bool {
    UNITS.iter().any(|unit| {
        key.strip_suffix(unit)
            .is_some_and(|stem| keys.iter().any(|other| other == stem))
    })
}

/// The keys of `table` its documentation does not mention.
fn undocumented(table: &Table, keys: &[String], configuration: &str) -> Vec<String> {
    let mut texts: Vec<String> = if table.sections.is_empty() {
        vec![configuration.to_owned()]
    } else {
        table
            .sections
            .iter()
            .filter_map(|heading| section(configuration, heading))
            .collect()
    };
    texts.extend(table.pages.iter().map(|name| page(name)));

    keys.iter()
        .filter(|key| {
            let documented = texts.iter().any(|text| documents(text, key));
            let older = is_older_name(key, keys) && documents(configuration, key);
            !(documented || older)
        })
        .cloned()
        .collect()
}

/// What is wrong with how `table` is documented, if anything.
fn problem(table: &Table, configuration: &str) -> Option<String> {
    let name = if table.header.is_empty() {
        "the top level"
    } else {
        table.header
    };
    if let Some(heading) = table
        .sections
        .iter()
        .find(|heading| section(configuration, heading).is_none())
    {
        return Some(format!(
            "{name}: configuration.md has no heading beginning {heading}"
        ));
    }
    let keys = match keys(table) {
        Ok(keys) => keys,
        Err(reason) => {
            return Some(format!(
                "{name}: probing with `{PROBE}` {reason}; is it still `deny_unknown_fields`?"
            ));
        }
    };
    let missing = undocumented(table, &keys, configuration);
    if missing.is_empty() {
        return None;
    }
    let mut pages = vec!["configuration.md"];
    pages.extend(table.pages);
    Some(format!(
        "{name}: `{}` not documented in {}",
        missing.join("`, `"),
        pages.join(" or ")
    ))
}

#[test]
fn every_key_a_factory_can_write_is_documented() {
    let configuration = page("configuration.md");
    let problems: Vec<String> = TABLES
        .iter()
        .filter_map(|table| problem(table, &configuration))
        .collect();

    assert!(
        problems.is_empty(),
        "the book is behind `layover.toml`:\n  - {}",
        problems.join("\n  - ")
    );
}
#[test]
fn every_way_serde_lists_what_it_expected_is_read() {
    assert_eq!(expected("expected `a`"), ["a"]);
    assert_eq!(expected("expected `a` or `b`"), ["a", "b"]);
    assert_eq!(expected("expected one of `a`, `b`, `c`"), ["a", "b", "c"]);
    assert!(expected("there are no fields").is_empty());
}

#[test]
fn a_key_is_documented_by_a_code_span_an_assignment_or_a_table_header_and_not_by_an_example() {
    let text = "Set `fuel_usd`, `window = \"24h\"`, `{ patch = \"24h\" }` or \
                `[agents.<name>.mcp.<server>]`.\n\n```toml\nmax_hops = 8\n```\n";

    assert!(documents(text, "fuel_usd"));
    assert!(documents(text, "window"));
    assert!(documents(text, "patch"));
    assert!(documents(text, "mcp"));
    assert!(!documents(text, "max_hops"));
    assert!(!documents(text, "fuel"));
    assert!(!documents(text, "dow"));
}

#[test]
fn a_section_ends_at_the_next_heading_and_not_at_a_comment_in_an_example() {
    let text = "## `[a]`\n`one`\n```toml\n# a comment\n```\n`two`\n### More\n`three`\n## `[b]`\n";
    let found = section(text, "## `[a]`").expect("the section is found");

    assert!(documents(&found, "one") && documents(&found, "two"));
    assert!(!documents(&found, "three"));
    assert!(section(text, "## `[c]`").is_none());
}
