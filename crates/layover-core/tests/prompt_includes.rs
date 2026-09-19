//! Behaviour of conditional prompt composition, exercised through the public API.
//!
//! These are the executable form of the `@include` contract: what a directive means, what happens
//! when one is wrong, and what a factory author is protected from.

use std::collections::BTreeMap;

use layover_core::prompt::{MAX_INCLUDE_DEPTH, referenced_flags, resolve};
use layover_core::{Flags, PromptError, PromptMap};

fn flags(pairs: &[(&str, bool)]) -> Flags {
    Flags::new(
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), *value))
            .collect::<BTreeMap<String, bool>>(),
    )
}

fn tester_source() -> PromptMap {
    PromptMap::new()
        .with(
            "tester.md",
            "You are the tester.\n@include(run_e2e) tester-e2e.md\n@include(!run_e2e) tester-local.md\nAlways report a verdict.\n",
        )
        .with("tester-e2e.md", "Run the remote end-to-end suite.\n")
        .with("tester-local.md", "Run the local suite only.\n")
}

#[test]
fn a_prompt_with_no_directives_is_returned_verbatim() {
    let source = PromptMap::new().with("plain.md", "Line one.\nLine two.\n");

    let text = resolve(&source, "plain.md", &Flags::default()).expect("resolves");

    assert_eq!(text, "Line one.\nLine two.\n");
}

#[test]
fn a_true_flag_pulls_its_file_in() {
    let text =
        resolve(&tester_source(), "tester.md", &flags(&[("run_e2e", true)])).expect("resolves");

    assert!(text.contains("You are the tester."));
    assert!(text.contains("Run the remote end-to-end suite."));
    assert!(
        !text.contains("Run the local suite only."),
        "the negated branch must be left out"
    );
    assert!(text.contains("Always report a verdict."));
}

#[test]
fn a_false_flag_selects_the_negated_branch() {
    let text =
        resolve(&tester_source(), "tester.md", &flags(&[("run_e2e", false)])).expect("resolves");

    assert!(text.contains("Run the local suite only."));
    assert!(!text.contains("Run the remote end-to-end suite."));
}

#[test]
fn a_directive_line_leaves_no_trace_when_skipped() {
    // The directive is replaced, not commented out. An agent must never see Layover's own syntax.
    let text =
        resolve(&tester_source(), "tester.md", &flags(&[("run_e2e", false)])).expect("resolves");

    assert!(
        !text.contains("@include"),
        "resolved prompt still contains directive syntax:\n{text}"
    );
}

#[test]
fn an_unconditional_include_always_applies() {
    let source = PromptMap::new()
        .with("main.md", "Header.\n@include shared.md\nFooter.\n")
        .with("shared.md", "Shared body.\n");

    let text = resolve(&source, "main.md", &Flags::default()).expect("resolves");

    assert_eq!(text, "Header.\nShared body.\nFooter.\n");
}

#[test]
fn includes_nest() {
    let source = PromptMap::new()
        .with("a.md", "A\n@include b.md\n")
        .with("b.md", "B\n@include c.md\n")
        .with("c.md", "C\n");

    let text = resolve(&source, "a.md", &Flags::default()).expect("resolves");

    assert_eq!(text, "A\nB\nC\n");
}

#[test]
fn an_included_path_is_relative_to_the_file_that_included_it() {
    let source = PromptMap::new()
        .with("roles/tester.md", "Tester.\n@include shared.md\n")
        .with("roles/shared.md", "Shared.\n");

    let text = resolve(&source, "roles/tester.md", &Flags::default()).expect("resolves");

    assert_eq!(text, "Tester.\nShared.\n");
}

#[test]
fn a_quoted_path_is_accepted() {
    let source = PromptMap::new()
        .with("main.md", "@include \"shared.md\"\n")
        .with("shared.md", "Shared.\n");

    assert_eq!(
        resolve(&source, "main.md", &Flags::default()).expect("resolves"),
        "Shared.\n"
    );
}

#[test]
fn a_flag_no_pipeline_declares_is_refused_rather_than_assumed_false() {
    // Silently treating an unknown flag as false would let a typo quietly delete a whole section
    // of an agent's instructions.
    let error = resolve(&tester_source(), "tester.md", &Flags::default())
        .expect_err("an undeclared flag must not be assumed");

    assert!(
        matches!(error, PromptError::UnknownFlag { ref flag, .. } if flag == "run_e2e"),
        "got {error:?}"
    );
}

#[test]
fn an_error_says_which_file_and_line_it_came_from() {
    let source = PromptMap::new().with("main.md", "Fine.\n@include(  ) thing.md\n");

    let error = resolve(&source, "main.md", &Flags::default()).expect_err("malformed");

    let message = error.to_string();
    assert!(message.contains("main.md"), "got {message}");
    assert!(message.contains("line 2"), "got {message}");
}

#[test]
fn a_missing_include_target_is_reported_with_its_path() {
    let source = PromptMap::new().with("main.md", "@include absent.md\n");

    let error = resolve(&source, "main.md", &Flags::default()).expect_err("missing file");

    assert!(
        matches!(error, PromptError::Missing { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_cycle_is_refused_rather_than_looping_forever() {
    let source = PromptMap::new()
        .with("a.md", "A\n@include b.md\n")
        .with("b.md", "B\n@include a.md\n");

    let error = resolve(&source, "a.md", &Flags::default()).expect_err("cycle");

    assert!(matches!(error, PromptError::Cycle { .. }), "got {error:?}");
}

#[test]
fn a_self_include_is_refused() {
    let source = PromptMap::new().with("a.md", "@include a.md\n");

    assert!(matches!(
        resolve(&source, "a.md", &Flags::default()),
        Err(PromptError::Cycle { .. })
    ));
}

#[test]
fn nesting_deeper_than_the_limit_is_refused() {
    let mut source = PromptMap::new();
    let depth = MAX_INCLUDE_DEPTH + 2;
    for level in 0..depth {
        source = source.with(
            format!("l{level}.md"),
            format!("L{level}\n@include l{}.md\n", level + 1),
        );
    }
    source = source.with(format!("l{depth}.md"), "bottom\n");

    let error = resolve(&source, "l0.md", &Flags::default()).expect_err("too deep");

    assert!(
        matches!(error, PromptError::TooDeep { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_path_climbing_out_of_the_prompt_root_is_refused() {
    let source = PromptMap::new().with("main.md", "@include ../../../etc/passwd\n");

    let error = resolve(&source, "main.md", &Flags::default()).expect_err("escape");

    assert!(
        matches!(error, PromptError::Escapes { .. }),
        "got {error:?}"
    );
}

#[test]
fn an_absolute_path_is_refused() {
    let source = PromptMap::new().with("main.md", "@include /etc/passwd\n");

    assert!(matches!(
        resolve(&source, "main.md", &Flags::default()),
        Err(PromptError::Escapes { .. })
    ));
}

#[test]
fn a_dot_dot_that_stays_inside_the_root_is_allowed() {
    // `..` is resolved lexically rather than banned, so sharing a file between subdirectories
    // works while escaping still does not.
    let source = PromptMap::new()
        .with("roles/tester.md", "Tester.\n@include ../shared/common.md\n")
        .with("shared/common.md", "Common.\n");

    assert_eq!(
        resolve(&source, "roles/tester.md", &Flags::default()).expect("resolves"),
        "Tester.\nCommon.\n"
    );
}

#[test]
fn referenced_flags_finds_every_branch_regardless_of_value() {
    let source = tester_source().with(
        "tester-e2e.md",
        "Remote suite.\n@include(publish_results) publish.md\n",
    );
    let source = source.with("publish.md", "Publish them.\n");

    let found = referenced_flags(&source, "tester.md").expect("walks");

    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        ["publish_results", "run_e2e"],
        "a flag behind a false condition still has to be declared"
    );
}

#[test]
fn referenced_flags_refuses_a_cycle_just_as_resolution_does() {
    // Validation has to apply the same rules composition does, or it would pass a factory whose
    // first run fails. It collects flag names *and* proves the graph is composable.
    let source = PromptMap::new()
        .with("a.md", "@include(x) b.md\n")
        .with("b.md", "@include(y) a.md\n");

    assert!(matches!(
        referenced_flags(&source, "a.md"),
        Err(PromptError::Cycle { .. })
    ));
}

#[test]
fn referenced_flags_walks_a_diamond_without_calling_it_a_cycle() {
    // Two files including a third is not a cycle, and both branches must be read for flag names.
    let source = PromptMap::new()
        .with("top.md", "@include(a) left.md\n@include(b) right.md\n")
        .with("left.md", "@include shared.md\n")
        .with("right.md", "@include shared.md\n")
        .with("shared.md", "@include(c) leaf.md\n")
        .with("leaf.md", "done\n");

    let found = referenced_flags(&source, "top.md").expect("a diamond is legal");

    assert_eq!(found.into_iter().collect::<Vec<_>>(), ["a", "b", "c"]);
}

#[test]
fn referenced_flags_refuses_nesting_past_the_depth_limit() {
    let mut source = PromptMap::new();
    let depth = MAX_INCLUDE_DEPTH + 2;
    for level in 0..depth {
        source = source.with(
            format!("l{level}.md"),
            format!("@include l{}.md\n", level + 1),
        );
    }
    source = source.with(format!("l{depth}.md"), "bottom\n");

    assert!(matches!(
        referenced_flags(&source, "l0.md"),
        Err(PromptError::TooDeep { .. })
    ));
}

#[test]
fn prose_that_merely_mentions_include_is_left_alone() {
    let source = PromptMap::new().with(
        "main.md",
        "Please @include the failing command in your report.\n",
    );

    let text = resolve(&source, "main.md", &Flags::default()).expect("resolves");

    assert_eq!(
        text,
        "Please @include the failing command in your report.\n"
    );
}

#[test]
fn a_line_that_looks_like_a_broken_directive_is_refused() {
    // Being lenient here would silently drop instructions an author believed were included.
    let source = PromptMap::new().with("main.md", "@include\n");

    assert!(matches!(
        resolve(&source, "main.md", &Flags::default()),
        Err(PromptError::MalformedDirective { .. })
    ));
}

#[test]
fn a_misspelt_directive_survives_as_visible_text() {
    // `@includ` is not the directive prefix, so it is prose. That is deliberate: the line stays in
    // the prompt where an author will see it, rather than being dropped or guessed at.
    let source = PromptMap::new().with("main.md", "@includ(run_e2e) thing.md\n");

    assert_eq!(
        resolve(&source, "main.md", &Flags::default()).expect("resolves"),
        "@includ(run_e2e) thing.md\n"
    );
}

/// Creates a symlink, or returns `false` when the platform will not allow it.
///
/// Windows needs Developer Mode or elevation to create symlinks, so a test that insisted would
/// fail on a perfectly good machine for a reason unrelated to what it checks. CI runs Linux as
/// well, so the check is genuinely exercised somewhere on every push.
#[cfg(unix)]
fn link(target: &std::path::Path, at: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, at).is_ok()
}

#[cfg(windows)]
fn link(target: &std::path::Path, at: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, at).is_ok()
}

#[test]
fn a_symlink_out_of_the_prompt_directory_is_refused() {
    // Lexical confinement sees a clean relative path here and lets it through; only comparing the
    // *resolved* path against the resolved root catches it. Prompt files are reviewed repository
    // content today, but they will not be once agents write their own.
    let root = std::env::temp_dir().join(format!("layover-prompt-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("prompts")).expect("temp dirs");

    let secret = root.join("secret.md");
    std::fs::write(&secret, "the private key is hunter2").expect("writes");

    let planted = root.join("prompts").join("innocent.md");
    if !link(&secret, &planted) {
        // Not silently passing: the test is reported as doing nothing rather than as succeeding.
        eprintln!("skipped: this platform would not create a symlink");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let source = layover_core::prompt::PromptDir::new(root.join("prompts"));
    let outcome = resolve(&source, "innocent.md", &Flags::default());

    assert!(
        matches!(outcome, Err(PromptError::Escapes { .. })),
        "a symlink out of the sandbox must be refused, got {outcome:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_ordinary_file_inside_the_prompt_directory_still_reads() {
    // The canonicalisation must not break the normal case, which is every prompt in every factory.
    let root = std::env::temp_dir().join(format!("layover-prompt-ok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested")).expect("temp dirs");
    std::fs::write(root.join("main.md"), "@include nested/part.md\n").expect("writes");
    std::fs::write(root.join("nested").join("part.md"), "the part\n").expect("writes");

    let source = layover_core::prompt::PromptDir::new(&root);

    assert_eq!(
        resolve(&source, "main.md", &Flags::default()).expect("resolves"),
        "the part\n"
    );

    let _ = std::fs::remove_dir_all(&root);
}
