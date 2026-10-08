use crate::config::{Runner, Selection};

fn none() -> Selection {
    Selection::default()
}

fn runner(args: &[&str]) -> Runner {
    toml::from_str(&format!(
        "command = [{}]",
        args.iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .expect("parses")
}

#[test]
fn a_model_placeholder_is_substituted_wherever_it_sits() {
    // Some CLIs take `--model x`, some take `--model=x`. The operator writes whichever theirs
    // wants, so substitution has to be textual rather than positional.
    let separate = runner(&["claude", "-p", "--model", "{model}"]);
    assert_eq!(
        separate.invocation(None, &Selection::model("claude-opus-5")),
        ["claude", "-p", "--model", "claude-opus-5"]
    );

    let joined = runner(&["codex", "exec", "--model={model}"]);
    assert_eq!(
        joined.invocation(None, &Selection::model("gpt-5.4")),
        ["codex", "exec", "--model=gpt-5.4"]
    );
}

#[test]
fn a_placeholder_with_no_option_before_it_disappears_alone() {
    // An empty string in argv is not nothing; several CLIs read it as a positional argument.
    let r = runner(&["agent", "{model}", "--verbose"]);
    assert_eq!(r.invocation(None, &none()), ["agent", "--verbose"]);
}

#[test]
fn an_unset_model_takes_its_option_with_it() {
    // It used to leave `--model` in front of the next flag, which Copilot CLI refuses outright
    // and Claude Code reads as the model's name.
    let r = runner(&[
        "claude",
        "-p",
        "--model",
        "{model}",
        "--output-format",
        "json",
    ]);
    assert_eq!(
        r.invocation(None, &none()),
        ["claude", "-p", "--output-format", "json"]
    );

    let joined = runner(&["codex", "exec", "--model={model}", "-"]);
    assert_eq!(
        joined.invocation(None, &none()),
        ["codex", "exec", "-"],
        "never the literal `--model={{model}}`"
    );
}

fn copilot() -> Runner {
    toml::from_str(
        r#"command = ["copilot", "--model", "{model}", "--reasoning-effort", "{effort}", "--context={context}", "{mcp}", "--allow-all-tools"]
mcp = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }"#,
    )
    .expect("parses")
}

fn chose(model: Option<&str>, effort: Option<&str>, context: Option<&str>) -> Selection {
    Selection {
        model: model.map(ToOwned::to_owned),
        effort: effort.map(ToOwned::to_owned),
        context: context.map(ToOwned::to_owned),
        ..Selection::default()
    }
}

#[test]
fn effort_and_context_are_substituted_alone_and_inside_an_argument() {
    let all = chose(Some("claude-opus-5.5"), Some("xhigh"), Some("long_context"));

    assert_eq!(
        copilot().invocation_with_mcp(None, &all, Some("/h/mcp.json")),
        [
            "copilot",
            "--model",
            "claude-opus-5.5",
            "--reasoning-effort",
            "xhigh",
            "--context=long_context",
            "--additional-mcp-config",
            "@/h/mcp.json",
            "--allow-all-tools",
        ]
    );
}

#[test]
fn codexs_config_key_carries_an_effort_and_leaves_with_its_option() {
    let codex = runner(&[
        "codex",
        "exec",
        "--model",
        "{model}",
        "-c",
        "model_reasoning_effort={effort}",
        "-",
    ]);

    assert_eq!(
        codex.invocation(None, &chose(Some("gpt-5.4"), Some("high"), None)),
        [
            "codex",
            "exec",
            "--model",
            "gpt-5.4",
            "-c",
            "model_reasoning_effort=high",
            "-"
        ]
    );
    assert_eq!(
        codex.invocation(None, &chose(Some("gpt-5.4"), None, None)),
        ["codex", "exec", "--model", "gpt-5.4", "-"],
        "a bare `-c` would swallow the `-` that reads the prompt"
    );
}

#[test]
fn an_unset_effort_or_context_leaves_no_trace_in_either_form() {
    assert_eq!(
        copilot().invocation_with_mcp(
            None,
            &chose(Some("claude-opus-5.5"), None, None),
            Some("/h/mcp.json")
        ),
        [
            "copilot",
            "--model",
            "claude-opus-5.5",
            "--additional-mcp-config",
            "@/h/mcp.json",
            "--allow-all-tools",
        ]
    );
}

#[test]
fn an_empty_value_counts_as_unset_rather_than_becoming_an_empty_argument() {
    assert_eq!(
        copilot().invocation(None, &chose(Some("m"), Some(""), Some("  "))),
        ["copilot", "--model", "m", "--allow-all-tools"]
    );
}

#[test]
fn no_combination_hands_the_cli_an_empty_argument_or_a_flag_without_its_value() {
    let valued = ["--model", "--reasoning-effort", "-c"];
    let r = runner(&[
        "agent",
        "--model",
        "{model}",
        "--reasoning-effort",
        "{effort}",
        "--context={context}",
        "-c",
        "model_reasoning_effort={effort}",
        "-",
    ]);

    for bits in 0..8_u8 {
        let pick = |bit: u8, value: &'static str| (bits & bit != 0).then_some(value);
        let selection = chose(pick(1, "m"), pick(2, "high"), pick(4, "long_context"));
        let argv = r.invocation(None, &selection);

        assert!(
            argv.iter()
                .all(|arg| !arg.trim().is_empty() && !arg.contains('{')),
            "{selection:?}: {argv:?}"
        );
        for (at, arg) in argv.iter().enumerate() {
            if valued.contains(&arg.as_str()) {
                let value = argv.get(at + 1).map_or("-", String::as_str);
                assert!(
                    !value.starts_with('-'),
                    "{selection:?}: `{arg}` has no value in {argv:?}"
                );
            }
        }
        assert_eq!(argv.last().map(String::as_str), Some("-"), "{argv:?}");
    }
}

#[test]
fn a_runner_says_which_values_it_can_carry() {
    assert!(copilot().takes_model());
    assert!(copilot().takes_effort());
    assert!(copilot().takes_context());

    let fixed = runner(&["copilot", "--reasoning-effort", "high"]);
    assert!(!fixed.takes_effort());
    assert!(!fixed.takes_context());
}

#[test]
fn a_value_fixed_beside_its_placeholder_is_found_in_every_shape() {
    let separated = runner(&[
        "copilot",
        "--reasoning-effort",
        "high",
        "--reasoning-effort",
        "{effort}",
    ]);
    assert_eq!(
        separated.fixes_beside(Runner::EFFORT).as_deref(),
        Some("high")
    );

    let joined = runner(&["copilot", "--context=default", "--context={context}"]);
    assert_eq!(
        joined.fixes_beside(Runner::CONTEXT).as_deref(),
        Some("default")
    );

    let mixed = runner(&["copilot", "--context", "default", "--context={context}"]);
    assert_eq!(
        mixed.fixes_beside(Runner::CONTEXT).as_deref(),
        Some("default")
    );

    let codex = runner(&[
        "codex",
        "-c",
        "model_reasoning_effort=low",
        "-c",
        "model_reasoning_effort={effort}",
        "-c",
        "sandbox=read-only",
    ]);
    assert_eq!(codex.fixes_beside(Runner::EFFORT).as_deref(), Some("low"));
}

#[test]
fn a_placeholder_alone_in_its_option_fixes_nothing() {
    assert_eq!(copilot().fixes_beside(Runner::EFFORT), None);
    assert_eq!(copilot().fixes_beside(Runner::CONTEXT), None);
    assert_eq!(copilot().fixes_beside(Runner::MODEL), None);

    // A different `-c` key is a different setting, not a second effort.
    let codex = runner(&[
        "codex",
        "-c",
        "sandbox=read-only",
        "-c",
        "model_reasoning_effort={effort}",
    ]);
    assert_eq!(codex.fixes_beside(Runner::EFFORT), None);
}

fn mcp_runner(args: &[&str], flag: &str) -> Runner {
    toml::from_str(&format!(
        "command = [{}]\nmcp = {{ flag = \"{flag}\", format = \"claude_json\" }}",
        args.iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .expect("parses")
}

#[test]
fn mcp_wiring_is_appended_when_the_command_does_not_place_it() {
    // What `claude` and `copilot` want, and what every existing factory file relies on.
    let r = mcp_runner(&["copilot", "--allow-all-tools"], "--mcp-config");
    assert_eq!(
        r.invocation_with_mcp(None, &none(), Some("/h/mcp.json")),
        [
            "copilot",
            "--allow-all-tools",
            "--mcp-config",
            "/h/mcp.json"
        ]
    );
}

#[test]
fn a_prefix_is_prepended_to_the_path_rather_than_passed_separately() {
    // Copilot CLI's `--additional-mcp-config` takes either a JSON string or a file path and
    // tells them apart by a leading `@`. Passed as its own argument the `@` would be a second
    // value the flag never sees; without it the path is parsed as JSON and the run dies
    // complaining about the factory's own configuration.
    let runner: Runner = toml::from_str(
        r#"command = ["copilot", "--allow-all-tools"]
mcp = { flag = "--additional-mcp-config", format = "claude_json", prefix = "@" }"#,
    )
    .expect("parses");

    assert_eq!(
        runner.invocation_with_mcp(None, &none(), Some("/h/mcp.json")),
        [
            "copilot",
            "--allow-all-tools",
            "--additional-mcp-config",
            "@/h/mcp.json"
        ]
    );
}

#[test]
fn a_prefix_applies_where_the_command_places_the_wiring_too() {
    // Both branches render the argument, and only one of them having the prefix would be a
    // factory that works until somebody adds `{mcp}` to keep a positional argument last.
    let runner: Runner = toml::from_str(
        r#"command = ["agent", "{mcp}", "-"]
mcp = { flag = "--cfg", format = "claude_json", prefix = "@" }"#,
    )
    .expect("parses");

    assert_eq!(
        runner.invocation_with_mcp(None, &none(), Some("/h/mcp.json")),
        ["agent", "--cfg", "@/h/mcp.json", "-"]
    );
}

#[test]
fn a_runner_without_a_prefix_still_gets_a_bare_path() {
    let runner = mcp_runner(&["claude", "-p"], "--mcp-config");

    assert_eq!(
        runner.invocation_with_mcp(None, &none(), Some("/h/mcp.json")),
        ["claude", "-p", "--mcp-config", "/h/mcp.json"]
    );
}

#[test]
fn mcp_wiring_goes_where_the_command_puts_it_when_it_says() {
    // `codex exec … -` reads the prompt from stdin and the `-` has to stay last, so appending
    // would put the flag after the argument it must precede.
    let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
    assert_eq!(
        r.invocation_with_mcp(None, &none(), Some("/h/mcp.toml")),
        ["codex", "exec", "-c", "/h/mcp.toml", "-"]
    );
}

#[test]
fn an_mcp_placeholder_disappears_when_there_is_nothing_to_wire() {
    // An unresolved placeholder reaching a CLI becomes an argument it does not understand.
    let r = mcp_runner(&["codex", "exec", "{mcp}", "-"], "-c");
    assert_eq!(
        r.invocation_with_mcp(None, &none(), None),
        ["codex", "exec", "-"]
    );
}

#[test]
fn a_runner_with_no_mcp_block_is_wired_to_nothing_even_if_a_path_exists() {
    // The factory always has a config file to offer; only the runner knows whether its CLI
    // can be told about one.
    let r = runner(&["echo", "hello"]);
    assert_eq!(
        r.invocation_with_mcp(None, &none(), Some("/h/mcp.json")),
        ["echo", "hello"]
    );
}

#[test]
fn the_prompt_path_is_substituted_independently_of_the_model() {
    let r = runner(&["agent", "--file", "{prompt}", "--model", "{model}"]);
    assert_eq!(
        r.invocation(Some("/run/prompt.md"), &Selection::model("m1")),
        ["agent", "--file", "/run/prompt.md", "--model", "m1"]
    );
}

#[test]
fn a_runner_without_placeholders_is_passed_through_untouched() {
    let r = runner(&["copilot", "--allow-all-tools"]);
    assert_eq!(
        r.invocation(Some("/x"), &Selection::model("m")),
        ["copilot", "--allow-all-tools"]
    );
    assert!(!r.takes_model());
    assert!(!r.takes_prompt_path());
}

#[test]
fn declaring_a_model_a_runner_cannot_carry_is_a_warning() {
    let config: crate::config::Config = toml::from_str(
        r#"
[layover]
work_dir = "work"

[defaults]
runner = "claude"

[runners.claude]
command = ["claude", "-p"]

[agents.analyst]
prompt = "analyse"
model = "claude-opus-5"
entry = true
"#,
    )
    .expect("parses");

    let said: Vec<_> = crate::validate::validate(&config)
        .iter()
        .map(|d| d.message.clone())
        .collect();

    assert!(
        said.iter().any(|m| m.contains("no `{model}` placeholder")),
        "{said:?}"
    );
}

fn preset(text: &str) -> Runner {
    toml::from_str(text).expect("parses")
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_owned()).collect()
}

#[test]
fn a_copilot_preset_runner_needs_only_what_it_denies() {
    // The factory that prompted this repeated these eleven arguments in each of five runners.
    let runner = preset(
        r#"cli  = "copilot"
args = ["--deny-tool=shell(git push)", "--deny-url=api.github.com"]"#,
    );
    let selection = Selection {
        args: args(&["--deny-tool=shell(orient reviews:*)"]),
        ..chose(Some("claude-opus-5.5"), Some("xhigh"), Some("long_context"))
    };

    assert_eq!(
        runner.invocation_with_mcp(None, &selection, Some("/h/mcp.json")),
        [
            "copilot",
            "--model=claude-opus-5.5",
            "--reasoning-effort=xhigh",
            "--context=long_context",
            "--allow-all-tools",
            "--allow-all-paths",
            "--allow-all-urls",
            "--no-ask-user",
            "--output-format",
            "json",
            "--deny-tool=shell(git push)",
            "--deny-url=api.github.com",
            "--deny-tool=shell(orient reviews:*)",
            "--additional-mcp-config",
            "@/h/mcp.json",
        ],
        "the preset, the runner's own denials, then the agent's, then the MCP wiring"
    );
    assert!(runner.takes_effort() && runner.takes_context() && runner.takes_model());
}

#[test]
fn a_named_chain_names_the_copilot_session_and_an_unnamed_one_names_nothing() {
    let runner = preset(r#"cli = "copilot""#);
    let named = Selection {
        name: Some("Login page: retry banner - bob".to_owned()),
        ..chose(Some("m"), None, None)
    };

    let line = runner.invocation(None, &named);
    assert!(
        line.iter()
            .any(|arg| arg == "--name=Login page: retry banner - bob"),
        "{line:?}"
    );
    assert!(
        runner
            .invocation(None, &chose(Some("m"), None, None))
            .iter()
            .all(|arg| !arg.starts_with("--name")),
    );
}

#[test]
fn a_claude_preset_reads_stdin_and_prints_what_a_cost_is_read_from() {
    let runner = preset(r#"cli = "claude""#);

    assert_eq!(
        runner.invocation_with_mcp(
            None,
            &chose(Some("claude-opus-5.5"), None, None),
            Some("/h/m.json")
        ),
        [
            "claude",
            "-p",
            "--model=claude-opus-5.5",
            "--output-format",
            "stream-json",
            "--verbose",
            "--mcp-config",
            "/h/m.json",
        ]
    );
    assert!(
        !runner.takes_effort(),
        "Claude Code takes no effort on its command line"
    );
}

#[test]
fn a_runners_own_mcp_wiring_replaces_its_presets() {
    let runner = preset(
        r#"cli = "copilot"
mcp = { flag = "--mcp", format = "claude_json" }"#,
    );

    let line = runner.invocation_with_mcp(None, &none(), Some("/h/mcp.json"));
    assert_eq!(&line[line.len() - 2..], ["--mcp", "/h/mcp.json"]);
}

#[test]
fn an_agents_args_go_where_a_command_says_or_at_the_end() {
    let placed = runner(&["codex", "exec", "{args}", "{mcp}", "-"]);
    let selection = Selection {
        args: args(&["--sandbox", "read-only"]),
        ..none()
    };
    assert_eq!(
        placed.invocation(None, &selection),
        ["codex", "exec", "--sandbox", "read-only", "-"]
    );

    let appended = runner(&["copilot", "--allow-all-tools"]);
    assert_eq!(
        appended.invocation(None, &selection),
        ["copilot", "--allow-all-tools", "--sandbox", "read-only"]
    );
    assert_eq!(
        placed.invocation(None, &none()),
        ["codex", "exec", "-"],
        "no args, no trace"
    );
}
