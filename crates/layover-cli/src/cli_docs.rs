//! `book/src/cli.md` against the parser it documents.
//!
//! The page is the reference for every command, and nothing else ties it to [`Cli`]: a flag added
//! to the parser without a word on the page is one only `--help` knows about. This walks the parser
//! rather than a list, so a new command or flag is something the page is asked about without anyone
//! deciding that it should be.
//!
//! It checks that each is mentioned, in its own command's section. Whether what is said about it is
//! still true is not something a test can know.

use clap::CommandFactory;

use super::Cli;

/// The long options clap gives every command by itself.
const BUILT_IN: &[&str] = &["help", "version"];

fn page() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../book/src/cli.md");
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
}

/// Each `` ## `name` `` section of `page`, with its text up to the next `## `.
fn sections(page: &str) -> Vec<(&str, String)> {
    let mut found: Vec<(&str, String)> = Vec::new();
    let mut fenced = false;
    for line in page.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && let Some(heading) = line.strip_prefix("## ") {
            let name = heading
                .strip_prefix('`')
                .and_then(|rest| rest.split_once('`'))
                .map_or("", |(name, _)| name);
            found.push((name, String::new()));
        } else if let Some((_, text)) = found.last_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    found.retain(|(name, _)| !name.is_empty());
    found
}

/// Whether `text` mentions `name` whole, rather than as the start of a longer one: `--now` is not
/// mentioned by `--nowhere`.
fn mentions(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        text[at + name.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || next == '-' || next == '_'))
    })
}

/// The long options of `command` a person can see and the page should explain.
fn options(command: &clap::Command) -> impl Iterator<Item = &str> {
    command
        .get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .filter_map(clap::Arg::get_long)
        .filter(|long| !BUILT_IN.contains(long))
}

/// Everything about `command`, reached as `layover <path>`, that the section `heading` of `text`
/// does not mention.
fn unmentioned(command: &clap::Command, path: &str, (heading, text): (&str, &str)) -> Vec<String> {
    let invocation = format!("layover {path}");
    let mut missing = Vec::new();
    if !mentions(text, &invocation) {
        missing.push(format!(
            "`{invocation}` is not in its ## `{heading}` section"
        ));
    }
    // A global option is not the command's own: it is explained once, for every command.
    for long in options(command).filter(|long| {
        !command
            .get_arguments()
            .any(|arg| arg.is_global_set() && arg.get_long() == Some(long))
    }) {
        if !mentions(text, &format!("--{long}")) {
            missing.push(format!(
                "`--{long}` of `{invocation}` is not in its ## `{heading}` section"
            ));
        }
    }
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        let path = format!("{path} {}", sub.get_name());
        missing.extend(unmentioned(sub, &path, (heading, text)));
    }
    missing
}

/// What the page leaves out of the parser, or says of a command the parser no longer has.
fn problems(page: &str) -> Vec<String> {
    let cli = Cli::command();
    let sections = sections(page);
    let mut problems = Vec::new();

    for long in options(&cli) {
        if !mentions(page, &format!("--{long}")) {
            problems.push(format!(
                "`--{long}`, an option of `layover` itself, is not on the page"
            ));
        }
    }
    let commands: Vec<&clap::Command> = cli
        .get_subcommands()
        .filter(|command| !command.is_hide_set())
        .collect();
    for command in &commands {
        let name = command.get_name();
        match sections.iter().find(|(heading, _)| *heading == name) {
            Some((_, text)) => problems.extend(unmentioned(command, name, (name, text))),
            None => problems.push(format!("`layover {name}` has no ## `{name}` section")),
        }
    }
    for (heading, _) in &sections {
        if !commands
            .iter()
            .any(|command| command.get_name() == *heading)
        {
            problems.push(format!(
                "## `{heading}` documents `layover {heading}`, which is not a command"
            ));
        }
    }
    problems
}
#[test]
fn every_command_and_option_is_in_the_book() {
    let problems = problems(&page());

    assert!(
        problems.is_empty(),
        "book/src/cli.md is out of step with the parser in src/main.rs:\n  - {}",
        problems.join("\n  - ")
    );
}

#[test]
fn a_mention_is_of_the_whole_name() {
    assert!(mentions("layover stop --now  # cut it off", "--now"));
    assert!(mentions("`layover update apply`", "layover update apply"));
    assert!(!mentions("--nowhere", "--now"));
    assert!(!mentions("--trust-checksum-only", "--trust-checksum"));
    assert!(!mentions("layover running", "layover run"));
}

#[test]
fn a_section_runs_from_its_heading_to_the_next_and_ignores_fenced_headings() {
    let page =
        "# Title\n## `serve`\n```sh\n## not a heading\n```\n--addr\n### Sub\n--now\n## `run`\nx\n";
    let found = sections(page);

    assert_eq!(found.len(), 2);
    assert_eq!(found[0].0, "serve");
    assert!(found[0].1.contains("--addr") && found[0].1.contains("--now"));
    assert_eq!((found[1].0, found[1].1.as_str()), ("run", "x\n"));
}
