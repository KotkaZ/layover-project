//! What each subcommand actually does.
//!
//! Every command returns its whole output as a string rather than printing as it goes, so the
//! behaviour can be tested without capturing stdout.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::{Timestamp, ToSpan};
use layover_core::cost::{RETENTION_DAYS, Window};
use layover_dashboard::{Dashboard, DashboardState};
use layover_store::{History, Journal, hangar, layout};
use layover_tower::{Dispatched, Factory};

use crate::mcp::ServedMcp;
use crate::tower::Tower;

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

/// Serves the monitoring dashboard, and runs the factory behind it, until interrupted.
///
/// Binds before printing the address, so the line it prints is a fact rather than a hope. The
/// configuration is validated first: a dashboard whose route map cannot be drawn is a confusing
/// way to find out the factory definition is broken.
///
/// This is the lights-out command. It fires scheduled pipelines, runs what is queued, and serves
/// agents the MCP endpoint they call back into. `--watch-only` leaves all of that out and serves a
/// read-only dashboard, which is what you want when pointing a second window at a factory another
/// process is already running.
///
/// # Errors
///
/// Returns [`Failure`] if the configuration is invalid, the history directory cannot be opened,
/// or the address is already in use.
pub fn serve(
    path: &Path,
    addr: &str,
    history: Option<&Path>,
    watch_only: bool,
    no_auth: bool,
) -> Result<String, Failure> {
    // Load once up front purely to fail early. A dashboard whose route map cannot be drawn is a
    // confusing way to discover the factory definition is broken.
    let (config, _) = load(path)?;

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
    let journal = Arc::new(
        Journal::open(history_dir.with_file_name("journal")).map_err(|error| error.to_string())?,
    );

    // Checked before anything reads or writes. A directory written by a newer release is refused
    // rather than read hopefully: an older build cannot know what it does not understand, and
    // writing it back without that would turn an afternoon's downgrade into permanent loss.
    match layout::open(&state_dir(path, history)) {
        Ok(layout::Opened::Current) => {}
        Ok(other) => println!("{other}"),
        Err(incompatible) => return Err(incompatible.to_string()),
    }

    // Enforce retention on the way up. `prune` and `sweep` existed and were called only from
    // tests, so the documented ninety days was a promise nothing kept: history grew forever while
    // the book said it did not. Doing it at startup rather than on a timer is enough for a
    // process meant to run continuously and be restarted when it is not.
    let horizon = Timestamp::now()
        .checked_sub((RETENTION_DAYS * 24).hours())
        .unwrap_or(Timestamp::MIN);
    store.prune(horizon).map_err(|error| error.to_string())?;
    journal.prune(horizon).map_err(|error| error.to_string())?;
    journal.sweep(horizon).map_err(|error| error.to_string())?;

    // Hangars were the one thing retention did not reach, which a 48-hour soak made visible: 1,501
    // runs left 3,050 files behind and nothing removed them. Worse than the size is what it meant
    // after ninety days — run records deleted while their transcripts remained, evidence attached
    // to runs nobody could look up any more.
    hangar::prune(&state_dir(path, history).join("hangars"), horizon)
        .map_err(|error| error.to_string())?;

    let dashboard = Dashboard::new(DashboardState {
        config_path: path.to_path_buf(),
        history: store,
        journal: Arc::clone(&journal),
        ground_stop: history_dir.with_file_name("ground-stop"),
    });

    // Held for the lifetime of the command. Dropping either stops it: the endpoint frees its port,
    // and the Tower finishes whatever run it is watching before the thread joins.
    let factory_root = path.parent().unwrap_or_else(|| Path::new("."));
    let running = if watch_only {
        None
    } else {
        let served = ServedMcp::start(&config, factory_root, &journal)?;

        let factory = Factory::new(config.clone(), factory_root)
            .map_err(|error| error.to_string())?
            .serving_mcp(served.endpoint.clone());

        served.resolve_against(factory.tokens());

        let tower = Tower::start(factory, Arc::clone(&journal), |line| println!("  {line}"))
            .map_err(|error| error.to_string())?;

        Some((served, tower))
    };

    // Copied out before the async block, which would otherwise take ownership of the pair and
    // Minted before anything binds, so the address that gets printed is the address that works.
    let guard = if no_auth {
        layover_dashboard::Guard::Open
    } else {
        layover_dashboard::Guard::minted()
    };
    let announced = guard.clone();

    // stop it being dropped — and therefore stopped — after the server returns.
    let endpoint = running.as_ref().map(|(served, _)| served.endpoint.clone());

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

        println!("Layover dashboard on {}", announced.address(&bound));
        println!("Reading {}", path.display());
        println!("History in {}", history_dir.display());

        if announced.token().is_some() {
            println!("The token is in that address; the page keeps it in a cookie afterwards.");
        } else {
            println!("Open to anything that can reach the port (--no-auth).");
        }

        match &endpoint {
            Some(url) => {
                println!("Agents reach Layover at {url}");
                println!("Running the factory: scheduled pipelines fire, queued work starts.");
            }
            // Said plainly, because a dashboard showing a queue that nothing will drain is the
            // failure this whole surface is meant to avoid.
            None => println!("Watching only: nothing here will start work (--watch-only)."),
        }

        // A token makes an off-loopback bind defensible; without one it is an open control plane
        // on a network. The warning is about the combination, not the address.
        if !bound.ip().is_loopback() && announced.token().is_none() {
            eprintln!();
            eprintln!("warning: {bound} is not loopback, and --no-auth is set.");
            eprintln!("         Anyone who can reach it can read run history, reports and help");
            eprintln!("         requests, and queue work this process will run. Drop --no-auth,");
            eprintln!("         or put something in front of it.");
        }

        println!("Press Ctrl+C to stop.");

        axum::serve(listener, layover_dashboard::router(dashboard, guard))
            .await
            .map_err(|error| error.to_string())
    })?;

    drop(running);

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
/// Checks a factory's recorded state and reports what a person should look at.
///
/// Returns the report and whether the factory would pass an unattended run, which the caller
/// turns into the exit code.
///
/// # Errors
///
/// Returns an error when the configuration or the state directory cannot be read.
pub fn doctor(path: &Path, window: &str) -> Result<(String, bool), Failure> {
    let (config, _) = load(path)?;

    let window = Window::from_slug(window).ok_or_else(|| {
        let known = Window::ALL
            .iter()
            .map(|w| w.slug())
            .collect::<Vec<_>>()
            .join(", ");
        format!("unknown window `{window}`. Known windows: {known}")
    })?;

    let root = path.parent().unwrap_or(Path::new("."));
    let report = crate::doctor::check(&config, root, window)?;

    Ok((report.render(), report.healthy()))
}

/// The state directory a command is about to use.
///
/// `--history` points at `.layover/history`, and the versioned thing is its parent: one marker
/// covers history, the journal, live run state and the Hangars, because they change shape
/// together and a per-file version would be four things to keep agreeing.
fn state_dir(config: &Path, history: Option<&Path>) -> PathBuf {
    match history {
        Some(dir) => dir
            .parent()
            .map_or_else(|| dir.to_path_buf(), Path::to_path_buf),
        None => config
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".layover"),
    }
}

/// Removes the `\\?\` prefix Windows canonicalisation adds.
fn strip_verbatim(path: &Path) -> PathBuf {
    let shown = path.display().to_string();
    match shown.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// Runs everything waiting in the queue, once.
///
/// # Errors
///
/// Returns a failure when the factory does not load or its state cannot be opened.
pub fn run(path: &Path, dry_run: bool) -> Result<String, Failure> {
    let (config, _) = load(path)?;
    let root = path.parent().unwrap_or_else(|| Path::new("."));

    match layout::open(&state_dir(path, None)) {
        Ok(layout::Opened::Current) => {}
        Ok(other) => println!("{other}"),
        Err(incompatible) => return Err(incompatible.to_string()),
    }

    let journal = Arc::new(
        Journal::open(root.join(".layover").join("journal")).map_err(|error| error.to_string())?,
    );
    let pending = journal.pending().map_err(|error| error.to_string())?;

    let mut out = String::new();

    if pending.is_empty() {
        return Ok("Nothing is queued.\n".to_owned());
    }

    if dry_run {
        let _ = writeln!(out, "{} flight(s) queued:", pending.len());
        for queued in &pending {
            let _ = writeln!(
                out,
                "  {} -> {}",
                queued.flight.id.as_str(),
                queued.flight.to
            );
        }
        let _ = writeln!(out, "\nNothing was started: --dry-run.");
        return Ok(out);
    }

    // Started before the factory, because the factory needs the address it actually bound to.
    // Port 0 and reading it back is the only way to be sure: a port chosen in advance can be
    // taken between choosing it and binding it, and a child told the wrong address fails in a way
    // that reads as the agent misbehaving.
    let served = ServedMcp::start(&config, root, &journal)?;

    let factory = Factory::new(config, root)
        .map_err(|error| error.to_string())?
        .serving_mcp(served.endpoint.clone());

    if factory.ground_stop_engaged() {
        return Err("a Ground Stop is engaged; release it before running anything".into());
    }

    served.resolve_against(factory.tokens());

    let mut lines = Vec::new();
    let drained = factory.drain_with(
        pending,
        &mut |flight| {
            // Off the queue before it runs. A flight that crashes the factory mid-run must not
            // come back on restart and do its work a second time.
            let _ = journal.unqueue(&flight.id);
        },
        &mut |flight, result| {
            lines.push(match result {
                Dispatched::Ran {
                    outcome,
                    usd,
                    source,
                } => {
                    // A figure's provenance is part of the figure. "$0.00 unmeasured" and
                    // "$0.00 measured" mean opposite things, and a line that shows only the
                    // number invites reading the first as the second.
                    let measured = if source.is_measured() {
                        "measured"
                    } else {
                        "not measured"
                    };
                    format!("  {} {outcome} ${usd:.2} ({measured})", flight.to)
                }
                other => format!("  {} {other}", flight.to),
            });
        },
        // Whatever the runs just finished put in the queue. Agents send flights while they run,
        // so the work waiting now is not the work that was waiting when this started.
        |_| journal.pending().unwrap_or_default(),
    );

    let _ = writeln!(out, "Ran {} flight(s):", drained.ran);
    for line in lines {
        let _ = writeln!(out, "{line}");
    }

    // Last and separate, because this is the failure that does not announce itself: work somebody
    // asked for that will not happen, and that nothing else will report.
    if !drained.abandoned.is_empty() {
        let _ = writeln!(out, "\nGave up on {} rendezvous:", drained.abandoned.len());
        for given_up in &drained.abandoned {
            let _ = writeln!(
                out,
                "  {given_up} ({} flight(s) stranded)",
                given_up.stranded
            );
        }
    }

    Ok(out)
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
        assert!(output.contains("9 agent(s)"), "{output}");
        assert!(output.contains("3 pipeline(s)"), "{output}");
    }

    #[test]
    fn every_shipped_example_validates_under_strict() {
        // The examples are the documentation people copy. One that warns teaches the warning is
        // normal, and one that errors teaches that validation is noise.
        for name in [
            "workitem-factory/layover.toml",
            "pr-review/layover.toml",
            "news-digest/layover.toml",
        ] {
            let output = validate_config(&example(name), true)
                .unwrap_or_else(|error| panic!("{name} does not validate: {error}"));

            assert!(output.contains("0 warning(s)"), "{name}: {output}");
        }
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
