//! How a runner invokes an agent CLI: its command, the placeholders substituted into it, and how
//! it is told where Layover's MCP server is.

use serde::Deserialize;

/// The values an agent's runner fills its `{model}`, `{effort}` and `{context}` placeholders with.
///
/// Each is passed through exactly as written. Layover keeps no catalog of models, or of the
/// efforts and context tiers each one accepts: the CLI does, and refuses a value it does not
/// support with a message that names it. A value that is empty or only whitespace counts as
/// unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// The model, such as `claude-opus-5.5`.
    pub model: Option<String>,
    /// How hard the model is asked to reason, such as `xhigh`.
    pub effort: Option<String>,
    /// The context-window tier, such as `long_context`.
    pub context: Option<String>,
}

impl Selection {
    /// A selection that names a model and nothing else.
    #[must_use]
    pub fn model(model: impl Into<String>) -> Self {
        Self {
            model: Some(model.into()),
            ..Self::default()
        }
    }

    /// What fills `placeholder`, or `None` when nothing does.
    fn value_for(&self, placeholder: &str) -> Option<&str> {
        let value = match placeholder {
            Runner::MODEL => &self.model,
            Runner::EFFORT => &self.effort,
            Runner::CONTEXT => &self.context,
            _ => return None,
        };
        value.as_deref().filter(|value| !value.trim().is_empty())
    }
}

/// How Layover passes its MCP endpoint to a runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpWiring {
    /// Command-line flag carrying the MCP configuration.
    pub flag: String,
    /// Configuration dialect the runner expects.
    pub format: String,
    /// Prepended to the path the flag carries.
    ///
    /// Copilot CLI's `--additional-mcp-config` takes *either* a JSON string or a file path, and
    /// tells the two apart by a leading `@`. Without it the path is read as JSON, which fails as
    /// a parse error about the factory's own configuration rather than anything recognisable.
    ///
    /// Empty for CLIs that take a plain path, which is most of them.
    #[serde(default)]
    pub prefix: String,
}

impl McpWiring {
    /// The argument that follows [`Self::flag`].
    #[must_use]
    pub fn argument(&self, path: &str) -> String {
        format!("{}{path}", self.prefix)
    }
}

/// How to invoke a particular headless agent CLI.
///
/// # A value that is not set
///
/// `{model}`, `{effort}` and `{context}` are optional: an agent that sets none of them runs on
/// whatever its CLI defaults to. So a placeholder with nothing to fill it must disappear cleanly,
/// and "cleanly" is the hard part. An argument that carries an unset placeholder is left out
/// whole — `--reasoning-effort={effort}` goes entirely, never as `--reasoning-effort=` or as the
/// literal text — and when that argument is the *value* of the option before it, the option goes
/// too: `"--reasoning-effort", "{effort}"` and `"-c", "model_reasoning_effort={effort}"` both
/// disappear as a pair. Every CLI that takes a value refuses a flag without one, so the
/// alternative is a run that dies at spawn — or, worse, one that reads the next flag as the
/// value.
///
/// "The option before it" means the argument immediately before, when that argument starts with
/// `-` and carries no `=` and no placeholder of its own. That is how every supported CLI pairs a
/// separate value with its flag, so the joined form (`--flag={effort}`) is the one to prefer: it
/// needs no pairing at all.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    /// Command and arguments.
    ///
    /// `{prompt}` substitutes the **path** to the agent's composed instructions, which the Tower
    /// writes into the run's Hangar before spawning. It is not the instructions themselves.
    ///
    /// That distinction is the whole design. A prompt is passed on **stdin**, never on the
    /// command line: Windows caps a command line at 32,767 characters, and real agent prompts go
    /// well past it — a sibling project's review agent composes to roughly 98 KB, three times
    /// over, and its ordinary developer agent to 34 KB. Inlining the prompt would work in every
    /// test written against a small fixture and fail on the first agent worth running.
    ///
    /// So most runners need no placeholder at all. It exists for CLIs that accept a file of
    /// instructions as a flag; those that do not get the instructions prepended to stdin.
    pub command: Vec<String>,
    /// How this runner is told where Layover's MCP server is.
    #[serde(default)]
    pub mcp: Option<McpWiring>,
}

impl Runner {
    /// The placeholder substituted with the path to the composed instructions.
    pub const PROMPT_PATH: &'static str = "{prompt}";

    /// The placeholder substituted with the agent's model.
    ///
    /// Every supported CLI spells its model flag differently — `--model`, `-m`, a config key — so
    /// the spelling stays in the runner command, which is already the one place that knows how to
    /// invoke a given CLI. The alternative, a `model_flag` field, would put half of an invocation
    /// in one place and half in another.
    pub const MODEL: &'static str = "{model}";

    /// The placeholder substituted with the agent's reasoning effort.
    ///
    /// For the same reason as [`Self::MODEL`]: Copilot CLI spells it `--reasoning-effort`, Codex
    /// `-c model_reasoning_effort=…`, and the spelling belongs with the rest of the invocation.
    pub const EFFORT: &'static str = "{effort}";

    /// The placeholder substituted with the agent's context-window tier, such as Copilot CLI's
    /// `--context long_context`.
    pub const CONTEXT: &'static str = "{context}";

    /// The placeholders an agent's [`Selection`] fills.
    const CHOSEN: [&'static str; 3] = [Self::MODEL, Self::EFFORT, Self::CONTEXT];

    /// The placeholder substituted with this runner's [`McpWiring::flag`] and the path to the
    /// generated MCP configuration — two arguments, not one.
    ///
    /// Optional. A command that does not contain it gets the pair appended at the end, which is
    /// what `claude` and `copilot` want. It exists for commands that end in a positional argument
    /// — `codex exec … -` reads the prompt from stdin and must stay last — where appending would
    /// put a flag after the thing it has to precede.
    pub const MCP: &'static str = "{mcp}";

    /// Returns `true` when this runner wants the instructions as a file it is handed.
    ///
    /// When `false`, the Tower prepends them to the stdin payload instead.
    #[must_use]
    pub fn takes_prompt_path(&self) -> bool {
        self.command
            .iter()
            .any(|arg| arg.contains(Self::PROMPT_PATH))
    }

    /// Returns `true` when this runner can carry an agent's `model`.
    ///
    /// An agent that declares a model whose runner cannot carry it is a silent no-op: the run
    /// happens, on whichever model the CLI defaults to, and nothing says the declaration was
    /// ignored. Validation warns about it rather than letting it pass.
    #[must_use]
    pub fn takes_model(&self) -> bool {
        self.takes(Self::MODEL)
    }

    /// Returns `true` when this runner can carry an agent's `effort`.
    ///
    /// Without the placeholder a declared effort is silently ignored, exactly as a model would be.
    #[must_use]
    pub fn takes_effort(&self) -> bool {
        self.takes(Self::EFFORT)
    }

    /// Returns `true` when this runner can carry an agent's `context`.
    #[must_use]
    pub fn takes_context(&self) -> bool {
        self.takes(Self::CONTEXT)
    }

    /// Returns `true` when some argument holds `placeholder`.
    #[must_use]
    pub fn takes(&self, placeholder: &str) -> bool {
        self.command.iter().any(|arg| arg.contains(placeholder))
    }

    /// A value this command fixes for the same option `placeholder` fills, if it fixes one.
    ///
    /// `--reasoning-effort high` beside `--reasoning-effort {effort}` hands the CLI two efforts,
    /// and which one wins depends on the CLI's last-wins rule and the order they happen to be
    /// written in — a runner that reads as configurable and is not. Recognises the three shapes a
    /// placeholder takes: standing alone after its option, joined to it with `=`, and as a
    /// `key=` value after an option such as Codex's `-c`.
    #[must_use]
    pub fn fixes_beside(&self, placeholder: &str) -> Option<String> {
        let args = &self.command;

        args.iter().enumerate().find_map(|(at, arg)| {
            let start = arg.find(placeholder)?;
            let before = &arg[..start];
            let previous = at
                .checked_sub(1)
                .map(|index| args[index].as_str())
                .filter(|previous| is_bare_option(previous));

            if arg == placeholder {
                fixed_value(args, previous?, "")
            } else if let Some(option) = before.strip_suffix('=').filter(|o| is_option(o)) {
                fixed_value(args, option, "")
            } else {
                fixed_value(args, previous?, before)
            }
        })
    }

    /// Builds the command line for one run.
    ///
    /// Substitution is textual and deliberately so: a placeholder sits inside an argument like
    /// `--model={model}` as readily as it stands alone, and the operator writes whichever their
    /// CLI expects. A placeholder with nothing to fill it is left out with the option it is the
    /// value of — see [`Runner`] — so no empty argument and no flag
    /// without its value reaches the CLI.
    #[must_use]
    pub fn invocation(&self, prompt_path: Option<&str>, selection: &Selection) -> Vec<String> {
        self.invocation_with_mcp(prompt_path, selection, None)
    }

    /// The command to run, with every placeholder resolved.
    ///
    /// `mcp_config` is the path to the file [`McpWiring::format`] describes. When the command
    /// names [`Self::MCP`] the flag and path replace it there; otherwise they are appended, which
    /// is the right answer for every CLI that does not end in a positional argument.
    #[must_use]
    pub fn invocation_with_mcp(
        &self,
        prompt_path: Option<&str>,
        selection: &Selection,
        mcp_config: Option<&str>,
    ) -> Vec<String> {
        let wiring = self.mcp.as_ref().zip(mcp_config);
        let mut out = Vec::with_capacity(self.command.len() + 2);
        // Whether the last argument kept is a bare option — `--model`, `-c` — whose value the next
        // argument may be.
        let mut after_option = false;

        for arg in &self.command {
            if arg == Self::MCP {
                if let Some((mcp, path)) = wiring {
                    out.push(mcp.flag.clone());
                    out.push(mcp.argument(path));
                }
                // Dropped when there is nothing to wire: an unresolved placeholder reaching a CLI
                // becomes an argument it does not understand.
                after_option = false;
                continue;
            }

            let unset = Self::CHOSEN.iter().any(|placeholder| {
                arg.contains(placeholder) && selection.value_for(placeholder).is_none()
            });
            if unset {
                if after_option && !is_option(arg) {
                    out.pop();
                }
                after_option = false;
                continue;
            }

            let mut rendered = arg.clone();
            if let Some(path) = prompt_path {
                rendered = rendered.replace(Self::PROMPT_PATH, path);
            }
            for placeholder in Self::CHOSEN {
                if let Some(value) = selection.value_for(placeholder) {
                    rendered = rendered.replace(placeholder, value);
                }
            }

            after_option = is_bare_option(arg);
            out.push(rendered);
        }

        if let Some((mcp, path)) = wiring
            && !self.command.iter().any(|arg| arg == Self::MCP)
        {
            out.push(mcp.flag.clone());
            out.push(mcp.argument(path));
        }

        out
    }
}

/// Whether `arg` is an option rather than a value: `--model`, `-c`, `--model=x` — but not `-`, which
/// is stdin, `--`, which ends options, or `-1`, which is a number.
fn is_option(arg: &str) -> bool {
    arg.len() > 1
        && arg.starts_with('-')
        && arg != "--"
        && !arg[1..].starts_with(|c: char| c.is_ascii_digit())
}

/// Whether `arg` is an option written without its value, so the argument after it may be the value.
fn is_bare_option(arg: &str) -> bool {
    is_option(arg) && !arg.contains('=') && !arg.contains('{')
}

/// A value `args` gives `option` literally — as `option value` or `option=value` — that starts with
/// `key`, without the `key`.
fn fixed_value(args: &[String], option: &str, key: &str) -> Option<String> {
    let joined = format!("{option}=");
    args.iter().enumerate().find_map(|(at, arg)| {
        let given = if arg == option {
            args.get(at + 1).map(String::as_str)
        } else {
            arg.strip_prefix(&joined)
        }?;
        let rest = given.strip_prefix(key)?;
        (!rest.is_empty() && !rest.contains('{') && !is_option(given)).then(|| rest.to_owned())
    })
}

#[cfg(test)]
mod tests {
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
}
