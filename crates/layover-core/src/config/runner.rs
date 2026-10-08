//! How a runner invokes an agent CLI: its command, the placeholders substituted into it, and how
//! it is told where Layover's MCP server is.

mod preset;

use serde::Deserialize;

pub use preset::Cli;

/// What fills a runner's placeholders for one run: `{model}`, `{effort}`, `{context}` and
/// `{name}`, and the arguments that take the place of `{args}`.
///
/// Each value is passed through exactly as written. Layover keeps no catalog of models, or of the
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
    /// What the run's session is called, from the name its chain was given.
    pub name: Option<String>,
    /// The agent's own arguments, and any its workflow adds, in that order.
    pub args: Vec<String>,
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
            Runner::NAME => &self.name,
            _ => return None,
        };
        value.as_deref().filter(|value| !value.trim().is_empty())
    }
}

/// How Layover passes its MCP endpoint to a runner.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
/// Written one of two ways. `cli = "copilot"` (or `"claude"`) has Layover supply everything an
/// unattended run of that CLI needs — see [`Cli`] — and the runner's `args` add what is particular
/// to it, which is usually what it denies. `command` spells out the whole invocation, for a CLI
/// Layover has no preset for or a runner that wants none of a preset's defaults.
///
/// # A value that is not set
///
/// `{model}`, `{effort}`, `{context}` and `{name}` are optional: an agent that sets none of them
/// runs on whatever its CLI defaults to. So a placeholder with nothing to fill it must disappear
/// cleanly, and "cleanly" is the hard part. An argument that carries an unset placeholder is left
/// out whole — `--reasoning-effort={effort}` goes entirely, never as `--reasoning-effort=` or as
/// the literal text — and when that argument is the *value* of the option before it, the option
/// goes too: `"--reasoning-effort", "{effort}"` and `"-c", "model_reasoning_effort={effort}"` both
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
    /// A CLI Layover knows, whose unattended invocation it supplies.
    ///
    /// Exactly one of this and [`Self::command`] is given; validation refuses neither and both.
    #[serde(default)]
    pub cli: Option<Cli>,
    /// The whole command and its arguments, for a runner with no [`Self::cli`].
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
    #[serde(default)]
    pub command: Vec<String>,
    /// Arguments added after what a [`Self::cli`] preset supplies: the runner's permission set,
    /// shared by every agent on it. Only with `cli`; a `command` says everything itself.
    #[serde(default)]
    pub args: Vec<String>,
    /// How this runner is told where Layover's MCP server is. A preset supplies it; given here,
    /// it replaces the preset's.
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

    /// The placeholder substituted with what the run's session is called: the name its chain was
    /// given and the agent, as in Copilot CLI's `--name`. Unset for a chain nobody named.
    pub const NAME: &'static str = "{name}";

    /// The placeholder an agent's own `args` take the place of — none, one or several arguments.
    ///
    /// Optional. Without it they are appended at the end, which is right for every CLI that does
    /// not end in a positional argument. A preset puts it last.
    pub const ARGS: &'static str = "{args}";

    /// The placeholders a [`Selection`] fills with a single value.
    const VALUES: [&'static str; 4] = [Self::MODEL, Self::EFFORT, Self::CONTEXT, Self::NAME];

    /// The placeholder substituted with this runner's [`McpWiring::flag`] and the path to the
    /// generated MCP configuration — two arguments, not one.
    ///
    /// Optional. A command that does not contain it gets the pair appended at the end, which is
    /// what `claude` and `copilot` want. It exists for commands that end in a positional argument
    /// — `codex exec … -` reads the prompt from stdin and must stay last — where appending would
    /// put a flag after the thing it has to precede.
    pub const MCP: &'static str = "{mcp}";

    /// What this runner runs before any agent's own arguments: a preset's whole invocation with the
    /// runner's `args`, or its `command` as written.
    #[must_use]
    pub fn template(&self) -> Vec<String> {
        match self.cli {
            Some(cli) => cli.template(&self.args),
            None => self.command.clone(),
        }
    }

    /// The template with `args` in place of `{args}`, or after everything when it has none.
    #[must_use]
    pub fn line_with(&self, args: &[String]) -> Vec<String> {
        let template = self.template();
        let mut line = Vec::with_capacity(template.len() + args.len());
        let mut placed = false;

        for arg in template {
            if arg == Self::ARGS {
                line.extend(args.iter().cloned());
                placed = true;
            } else {
                line.push(arg);
            }
        }
        if !placed {
            line.extend(args.iter().cloned());
        }
        line
    }

    /// How this runner tells its CLI where Layover's MCP server is: its own `mcp`, or its preset's.
    #[must_use]
    pub fn wiring(&self) -> Option<McpWiring> {
        self.mcp.clone().or_else(|| self.cli.map(Cli::wiring))
    }

    /// Returns `true` when this runner wants the instructions as a file it is handed.
    ///
    /// When `false`, the Tower prepends them to the stdin payload instead.
    #[must_use]
    pub fn takes_prompt_path(&self) -> bool {
        self.takes(Self::PROMPT_PATH)
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
        self.template().iter().any(|arg| arg.contains(placeholder))
    }

    /// A value this runner fixes for the same option `placeholder` fills, if it fixes one. See
    /// [`fixes_beside`].
    #[must_use]
    pub fn fixes_beside(&self, placeholder: &str) -> Option<String> {
        fixes_beside(&self.template(), placeholder)
    }

    /// Builds the command line for one run.
    ///
    /// Substitution is textual and deliberately so: a placeholder sits inside an argument like
    /// `--model={model}` as readily as it stands alone, and the operator writes whichever their
    /// CLI expects. A placeholder with nothing to fill it is left out with the option it is the
    /// value of — see [`Runner`] — so no empty argument and no flag without its value reaches the
    /// CLI.
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
        let line = self.line_with(&selection.args);
        let wiring = self.wiring();
        let wiring = wiring.as_ref().zip(mcp_config);
        let mut out = Vec::with_capacity(line.len() + 2);
        // Whether the last argument kept is a bare option — `--model`, `-c` — whose value the next
        // argument may be.
        let mut after_option = false;

        for arg in &line {
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

            let unset = Self::VALUES.iter().any(|placeholder| {
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
            for placeholder in Self::VALUES {
                if let Some(value) = selection.value_for(placeholder) {
                    rendered = rendered.replace(placeholder, value);
                }
            }

            after_option = is_bare_option(arg);
            out.push(rendered);
        }

        if let Some((mcp, path)) = wiring
            && !line.iter().any(|arg| arg == Self::MCP)
        {
            out.push(mcp.flag.clone());
            out.push(mcp.argument(path));
        }

        out
    }
}

/// A value `line` fixes for the same option `placeholder` fills, if it fixes one.
///
/// `--reasoning-effort high` beside `--reasoning-effort {effort}` hands the CLI two efforts, and
/// which one wins depends on the CLI's last-wins rule and the order they happen to be written in —
/// a runner that reads as configurable and is not. Recognises the three shapes a placeholder takes:
/// standing alone after its option, joined to it with `=`, and as a `key=` value after an option
/// such as Codex's `-c`.
#[must_use]
pub fn fixes_beside(line: &[String], placeholder: &str) -> Option<String> {
    line.iter().enumerate().find_map(|(at, arg)| {
        let start = arg.find(placeholder)?;
        let before = &arg[..start];
        let previous = at
            .checked_sub(1)
            .map(|index| line[index].as_str())
            .filter(|previous| is_bare_option(previous));

        if arg == placeholder {
            fixed_value(line, previous?, "")
        } else if let Some(option) = before.strip_suffix('=').filter(|o| is_option(o)) {
            fixed_value(line, option, "")
        } else {
            fixed_value(line, previous?, before)
        }
    })
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
mod tests;
