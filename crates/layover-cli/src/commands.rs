//! What each subcommand actually does.
//!
//! Every command returns its whole output as a string rather than printing as it goes, so the
//! behaviour can be tested without capturing stdout.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use layover_dashboard::{Dashboard, DashboardState};
use layover_store::{History, Journal};

use layover_core::agent::PromptSpec;
use layover_core::prompt::resolve;
use layover_core::{
    AgentName, Autostart, Config, Diagnostic, Flags, PipelineName, Platform, PromptDir, Severity,
    Trigger, validate, validate_prompts,
};

/// Anything that stops a command from finishing.
pub type Failure = String;

/// Loads a factory definition and the prompt source that goes with it.
///
/// `prompt_dir` is resolved relative to the configuration file, so a factory can be run from
/// anywhere without its prompt paths changing meaning.
fn load(path: &Path) -> Result<(Config, PromptDir), Failure> {
    let config = Config::load(path).map_err(|error| describe(&error))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let source = PromptDir::new(base.join(&config.layover.prompt_dir));
    Ok((config, source))
}

/// Renders an error and the chain of causes beneath it.
///
/// `ConfigError` says *what* failed; the source underneath says *why*. Printing only the outer
/// message would tell an author their file is broken without telling them where.
fn describe(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let _ = write!(message, "\n  caused by: {cause}");
        source = cause.source();
    }
    message
}

/// Checks a factory definition and reports everything wrong with it.
///
/// # Errors
///
/// Returns the findings when any of them would block startup, or when `strict` is set and there
/// are warnings.
pub fn validate_config(path: &Path, strict: bool) -> Result<String, Failure> {
    let (config, source) = load(path)?;

    let mut found = validate(&config);
    found.extend(validate_prompts(&config, &source));

    let errors = found.iter().filter(|d| d.is_error()).count();
    let warnings = found.len() - errors;

    let mut out = String::new();
    for diagnostic in &found {
        let _ = writeln!(out, "{}", render(diagnostic));
    }

    if errors > 0 {
        let _ = write!(
            out,
            "\n{errors} error(s), {warnings} warning(s) — this factory will not start"
        );
        return Err(out);
    }

    if strict && warnings > 0 {
        let _ = write!(out, "\n{warnings} warning(s), and --strict was given");
        return Err(out);
    }

    let _ = writeln!(
        out,
        "{} agent(s), {} pipeline(s), {} route(s) — {warnings} warning(s), no errors",
        config.agents.len(),
        config.pipelines.len(),
        config.routes.len()
    );
    Ok(out)
}

fn render(diagnostic: &Diagnostic) -> String {
    let label = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    format!("{label}: {}", diagnostic.message)
}

/// Describes the factory in the terms an operator thinks in.
///
/// # Errors
///
/// Returns a message when the definition cannot be loaded.
/// Prints the factory's route map, as Mermaid source or as SVG.
///
/// No live state: the CLI is not talking to a running Tower, so this is the topology alone. The
/// dashboard renders the same functions with the current activity overlaid.
///
/// # Errors
///
/// Returns [`Failure`] if the configuration cannot be loaded.
pub fn graph(path: &Path, svg: bool) -> Result<String, Failure> {
    let (config, _) = load(path)?;
    let live = layover_core::diagram::Live::default();

    Ok(if svg {
        layover_core::diagram::render_svg(&layover_core::diagram::Layout::build(&config, &live))
    } else {
        layover_core::diagram::route_map(&config, &live)
    })
}

/// Serves the monitoring dashboard until interrupted.
///
/// Binds before printing the address, so the line it prints is a fact rather than a hope. The
/// configuration is validated first: a dashboard whose route map cannot be drawn is a confusing
/// way to find out the factory definition is broken.
///
/// # Errors
///
/// Returns [`Failure`] if the configuration is invalid, the history directory cannot be opened,
/// or the address is already in use.
pub fn serve(path: &Path, addr: &str, history: Option<&Path>) -> Result<String, Failure> {
    // Load once up front purely to fail early. A dashboard whose route map cannot be drawn is a
    // confusing way to discover the factory definition is broken.
    load(path)?;

    let history_dir = history.map_or_else(
        || {
            path.parent()
                .unwrap_or_else(|| Path::new("."))
                .join(".layover")
                .join("history")
        },
        Path::to_path_buf,
    );

    let store = History::open(&history_dir).map_err(|error| error.to_string())?;
    let journal =
        Journal::open(history_dir.with_file_name("journal")).map_err(|error| error.to_string())?;
    let dashboard = Dashboard::new(DashboardState {
        config_path: path.to_path_buf(),
        history: store,
        journal,
    });

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;

    runtime.block_on(async move {
        // Bind before announcing, so the address printed is a fact rather than a hope.
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|error| format!("could not listen on {addr}: {error}"))?;
        let bound = listener.local_addr().map_err(|error| error.to_string())?;

        println!("Layover dashboard on http://{bound}");
        println!("Reading {}", path.display());
        println!("History in {}", history_dir.display());
        println!("Press Ctrl+C to stop.");

        axum::serve(listener, layover_dashboard::router(dashboard))
            .await
            .map_err(|error| error.to_string())
    })?;

    Ok(String::new())
}

pub fn explain(path: &Path) -> Result<String, Failure> {
    let (config, _) = load(path)?;
    let mut out = String::new();

    out.push_str("Pipelines\n");
    if config.pipelines.is_empty() {
        out.push_str("  (none declared; only agents marked `entry = true` can be triggered)\n");
    }
    for (name, pipeline) in &config.pipelines {
        let trigger = match &pipeline.trigger {
            Trigger::Manual => "manual".to_owned(),
            Trigger::Scheduled(schedule) => schedule.to_string(),
        };
        let _ = writeln!(out, "  {name} [{trigger}] -> {}", pipeline.entry);
        if let Some(description) = &pipeline.description {
            let _ = writeln!(out, "      {description}");
        }
        for (flag, spec) in &pipeline.flags {
            let _ = writeln!(
                out,
                "      --flag {flag}={} {}",
                spec.default,
                spec.description.as_deref().unwrap_or("")
            );
        }
    }

    out.push_str("\nAgents\n");
    for (name, agent) in &config.agents {
        let _ = writeln!(
            out,
            "  {name} [{}] {}",
            match agent.access {
                layover_core::Access::ReadOnly => "read-only",
                layover_core::Access::ReadWrite => "read-write",
            },
            agent.description_or_placeholder()
        );
    }

    out.push_str("\nRoutes\n");
    for route in &config.routes {
        let from = join(&route.from);
        let to = join(&route.to);
        let annotation = match route.join {
            Some(condition) => format!("  [join = {condition:?}]").to_lowercase(),
            None => String::new(),
        };
        let _ = writeln!(out, "  {from} -> {to}{annotation}");
    }

    Ok(out)
}

fn join(names: &[AgentName]) -> String {
    names
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders an agent's prompt exactly as a run would receive it.
///
/// # Errors
///
/// Returns a message when the agent is unknown, the pipeline is unknown, a flag override is
/// malformed, or the prompt does not compose.
pub fn prompt(
    path: &Path,
    agent: &str,
    pipeline: Option<&str>,
    overrides: &[String],
) -> Result<String, Failure> {
    let (config, source) = load(path)?;

    let name = AgentName::from(agent);
    let Some(definition) = config.agents.get(&name) else {
        return Err(format!(
            "unknown agent `{agent}`; this factory declares {}",
            join(&config.agents.keys().cloned().collect::<Vec<_>>())
        ));
    };

    let overrides = parse_flags(overrides)?;
    let flags = resolve_flags(&config, pipeline, overrides)?;

    match definition.prompt_spec() {
        Err(error) => Err(format!("agent `{agent}`: {error}")),
        Ok(PromptSpec::Inline(text)) => Ok(text),
        Ok(PromptSpec::File(file)) => {
            resolve(&source, &file, &flags).map_err(|error| format!("agent `{agent}`: {error}"))
        }
    }
}

fn parse_flags(overrides: &[String]) -> Result<BTreeMap<String, bool>, Failure> {
    let mut parsed = BTreeMap::new();

    for raw in overrides {
        let Some((name, value)) = raw.split_once('=') else {
            return Err(format!("could not read `{raw}`; expected `name=true`"));
        };
        let value = match value.trim() {
            "true" => true,
            "false" => false,
            other => {
                return Err(format!(
                    "flag `{name}` was given `{other}`; expected `true` or `false`"
                ));
            }
        };
        parsed.insert(name.trim().to_owned(), value);
    }

    Ok(parsed)
}

/// Works out the flag values a run would see.
///
/// With a pipeline named, its declared defaults apply. Without one, every flag any pipeline
/// declares is offered at its default, so `layover prompt` works on a factory with several
/// pipelines without forcing the caller to pick one.
fn resolve_flags(
    config: &Config,
    pipeline: Option<&str>,
    overrides: BTreeMap<String, bool>,
) -> Result<Flags, Failure> {
    if let Some(name) = pipeline {
        let key = PipelineName::from(name);
        let Some(pipeline) = config.pipelines.get(&key) else {
            return Err(format!("unknown pipeline `{name}`"));
        };
        return pipeline
            .flags_for_run(&overrides)
            .map_err(|error| error.to_string());
    }

    let mut values: BTreeMap<String, bool> = BTreeMap::new();
    for pipeline in config.pipelines.values() {
        for (flag, spec) in &pipeline.flags {
            values.entry(flag.clone()).or_insert(spec.default);
        }
    }

    for (flag, value) in overrides {
        if !values.contains_key(&flag) {
            return Err(format!("no pipeline declares flag `{flag}`"));
        }
        values.insert(flag, value);
    }

    Ok(Flags::new(values))
}

/// Writes the file that starts Layover at logon.
///
/// # Errors
///
/// Returns a message when the platform has no supported mechanism, the configuration cannot be
/// loaded, or the file cannot be written.
pub fn autostart(config: &Path, output: Option<&Path>, show: bool) -> Result<String, Failure> {
    // Load the configuration even though only its path is used: writing an autostart entry for a
    // factory that does not parse would produce a service that fails at every logon.
    let (_, _) = load(config)?;

    let Some(platform) = Platform::current() else {
        return Err(format!(
            "no autostart mechanism for `{}`; run Layover under a supervisor of your own",
            std::env::consts::OS
        ));
    };

    let binary = std::env::current_exe()
        .map_err(|error| format!("could not find this executable: {error}"))?;
    let config = config.canonicalize().map_or_else(
        |_| config.to_path_buf(),
        |path| strip_verbatim(path.as_path()),
    );

    let entry = Autostart::new(strip_verbatim(&binary), &config);
    let rendered = entry.render(platform);

    if show {
        return Ok(rendered);
    }

    let target = match output {
        Some(path) => path.to_path_buf(),
        None => config
            .parent()
            .unwrap_or(Path::new("."))
            .join(platform.artefact_name()),
    };

    std::fs::write(&target, &rendered)
        .map_err(|error| format!("could not write {}: {error}", target.display()))?;

    Ok(format!(
        "Wrote a {platform} to {}\n\nRegister it with:\n\n{}\n",
        target.display(),
        platform.install_hint(&target)
    ))
}

/// Removes Windows' `\\?\` verbatim prefix.
///
/// `canonicalize` produces it on Windows, and `schtasks` rejects a path that carries it — with an
/// error naming neither the path nor the prefix.
fn strip_verbatim(path: &Path) -> PathBuf {
    let shown = path.display().to_string();
    match shown.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(name)
    }

    #[test]
    fn the_reference_factory_validates() {
        let output = validate_config(&example("workitem-factory/layover.toml"), true)
            .expect("the reference factory must be clean even under --strict");

        assert!(output.contains("no errors"), "{output}");
        assert!(output.contains("8 agent(s)"), "{output}");
        assert!(output.contains("2 pipeline(s)"), "{output}");
    }

    #[test]
    fn the_minimal_example_validates() {
        assert!(validate_config(&example("planner.toml"), true).is_ok());
    }

    #[test]
    fn a_missing_file_explains_itself() {
        let error = validate_config(Path::new("no-such-factory.toml"), false)
            .expect_err("a missing file is an error");

        assert!(error.contains("could not read"), "{error}");
        assert!(
            error.contains("caused by"),
            "the underlying reason must be shown: {error}"
        );
    }

    #[test]
    fn explain_lists_pipelines_agents_and_routes() {
        let output = explain(&example("workitem-factory/layover.toml")).expect("explains");

        assert!(
            output.contains("review-bot [every 3600s] -> pr_scanner"),
            "{output}"
        );
        assert!(
            output.contains("development [manual] -> analyst"),
            "{output}"
        );
        assert!(output.contains("--flag run_e2e=false"), "{output}");
        assert!(output.contains("tester [read-only]"), "{output}");
        assert!(output.contains("[join = all]"), "{output}");
    }

    #[test]
    fn a_prompt_is_rendered_with_pipeline_defaults() {
        let text = prompt(
            &example("workitem-factory/layover.toml"),
            "tester",
            Some("development"),
            &[],
        )
        .expect("renders");

        assert!(text.contains("Local suite only"));
        assert!(!text.contains("@include"));
    }

    #[test]
    fn a_flag_override_changes_the_rendered_prompt() {
        let text = prompt(
            &example("workitem-factory/layover.toml"),
            "tester",
            Some("development"),
            &["run_e2e=true".to_owned()],
        )
        .expect("renders");

        assert!(text.contains("End-to-end suite"));
        assert!(!text.contains("Local suite only"));
    }

    #[test]
    fn a_prompt_works_without_naming_a_pipeline() {
        let text = prompt(
            &example("workitem-factory/layover.toml"),
            "publisher",
            None,
            &[],
        )
        .expect("renders");

        assert!(text.contains("Draft pull request"), "{text}");
    }

    #[test]
    fn an_inline_prompt_is_returned_as_written() {
        let text = prompt(&example("planner.toml"), "coder", None, &[]).expect("renders");

        assert_eq!(
            text,
            "You implement the task described in the incoming flight."
        );
    }

    #[test]
    fn an_unknown_agent_lists_the_real_ones() {
        let error = prompt(&example("planner.toml"), "ghost", None, &[])
            .expect_err("unknown agent is an error");

        assert!(error.contains("unknown agent `ghost`"), "{error}");
        assert!(error.contains("planner"), "{error}");
    }

    #[test]
    fn an_unknown_pipeline_is_an_error() {
        let error = prompt(&example("planner.toml"), "coder", Some("nope"), &[])
            .expect_err("unknown pipeline is an error");

        assert!(error.contains("unknown pipeline `nope`"), "{error}");
    }

    #[test]
    fn a_malformed_flag_is_an_error() {
        assert!(parse_flags(&["run_e2e".to_owned()]).is_err());
        assert!(parse_flags(&["run_e2e=yes".to_owned()]).is_err());
        assert_eq!(
            parse_flags(&["run_e2e=true".to_owned(), "draft_pr=false".to_owned()]),
            Ok(BTreeMap::from([
                ("run_e2e".to_owned(), true),
                ("draft_pr".to_owned(), false),
            ]))
        );
    }

    #[test]
    fn an_undeclared_flag_override_is_an_error() {
        let error = prompt(
            &example("workitem-factory/layover.toml"),
            "tester",
            None,
            &["nonesuch=true".to_owned()],
        )
        .expect_err("an undeclared flag is an error");

        assert!(error.contains("nonesuch"), "{error}");
    }

    #[test]
    fn autostart_renders_without_writing_anything() {
        let text = autostart(&example("workitem-factory/layover.toml"), None, true)
            .expect("renders for this platform");

        assert!(text.contains("layover.toml"), "{text}");
        assert!(
            !text.contains(r"\\?\"),
            "the Windows verbatim prefix breaks schtasks: {text}"
        );
    }

    #[test]
    fn autostart_refuses_a_factory_that_does_not_load() {
        // Registering a service for a broken factory would fail silently at every logon.
        assert!(autostart(Path::new("no-such-factory.toml"), None, true).is_err());
    }
}
