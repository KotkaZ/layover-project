//! Turning domain types into the shapes the API promises.
//!
//! Separate from the handlers so that the mapping can be read in one place, and so that a field
//! added to the specification produces a compile error here rather than a silently absent value
//! somewhere in a handler.

use layover_core::agent::Access as CoreAccess;
use layover_core::config::Config;
use layover_core::cost::{CostSource as CoreSource, Span, Summary, Window};
use layover_core::pipeline::{Schedule, Trigger as CoreTrigger};
use layover_core::route::Join as CoreJoin;
use layover_core::run::{Outcome, RunRecord};
use layover_http::{
    Access, Agent, AgentList, CostSource, CostSummary, CostWindow, Flag, Join, Pipeline,
    PipelineList, Route, Run, RunStatus, TokenUsage, Trigger, TriggerKind, WindowSpan,
};

/// Describes the factory's agents and the edges between them.
pub fn agents(config: &Config) -> AgentList {
    AgentList {
        agents: config
            .agents
            .iter()
            .map(|(name, agent)| Agent {
                name: name.to_string(),
                description: agent.description.clone(),
                purpose: agent.purpose.clone(),
                runner: agent.runner.clone(),
                model: agent.model.clone(),
                access: match agent.access {
                    CoreAccess::ReadOnly => Access::ReadOnly,
                    CoreAccess::ReadWrite => Access::ReadWrite,
                },
                entry: agent.entry,
                resident: agent.resident,
            })
            .collect(),
        routes: config
            .routes
            .iter()
            .map(|route| Route {
                from: route.from.iter().map(ToString::to_string).collect(),
                to: route.to.iter().map(ToString::to_string).collect(),
                join: route.join.map(|join| match join {
                    CoreJoin::All => Join::All,
                    CoreJoin::Any => Join::Any,
                }),
                timeout_sec: route.timeout_sec.and_then(|secs| i64::try_from(secs).ok()),
            })
            .collect(),
    }
}

/// Describes the ways into the factory.
pub fn pipelines(config: &Config) -> PipelineList {
    PipelineList {
        pipelines: config
            .pipelines
            .iter()
            .map(|(name, pipeline)| Pipeline {
                name: name.to_string(),
                description: pipeline.description.clone(),
                entry: pipeline.entry.to_string(),
                trigger: trigger(&pipeline.trigger),
                flags: pipeline
                    .flags
                    .iter()
                    .map(|(name, spec)| Flag {
                        name: name.clone(),
                        description: spec.description.clone(),
                        default: spec.default,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Describes what starts a pipeline.
fn trigger(trigger: &CoreTrigger) -> Trigger {
    match trigger {
        CoreTrigger::Manual => Trigger {
            kind: TriggerKind::Manual,
            every_seconds: None,
            cron: None,
        },
        CoreTrigger::Scheduled(Schedule::Every(interval)) => Trigger {
            kind: TriggerKind::Scheduled,
            every_seconds: i64::try_from(interval.as_secs()).ok(),
            cron: None,
        },
        CoreTrigger::Scheduled(Schedule::Cron(expression)) => Trigger {
            kind: TriggerKind::Scheduled,
            every_seconds: None,
            cron: Some(expression.clone()),
        },
    }
}

/// Describes one run.
pub fn run(record: &RunRecord) -> Run {
    Run {
        run_id: record.run.to_string(),
        itinerary_id: record.itinerary.as_str().to_owned(),
        agent: record.agent.to_string(),
        pipeline: record.pipeline.as_ref().map(ToString::to_string),
        model: record.model.clone(),
        status: status(record.outcome),
        started_at: record.started_at.to_string(),
        finished_at: record.finished_at.map(|at| at.to_string()),
        duration_sec: record.duration_secs(),
        exit_code: None,
        // Null rather than zero when nothing was reported. A zero would be indistinguishable
        // from a run that genuinely cost nothing, and the difference decides whether the budget
        // rail is working.
        cost_usd: (record.source != CoreSource::Unreported).then_some(record.usd),
        cost_source: source(record.source),
        detail: record.detail.clone(),
        hops_remaining: None,
    }
}

/// Describes how a run ended.
fn status(outcome: Outcome) -> RunStatus {
    match outcome {
        Outcome::Running => RunStatus::Running,
        Outcome::Succeeded => RunStatus::Succeeded,
        Outcome::Failed => RunStatus::Failed,
        Outcome::TimedOut => RunStatus::TimedOut,
        Outcome::Halted => RunStatus::Halted,
        Outcome::Interrupted => RunStatus::Interrupted,
    }
}

/// Describes where a cost figure came from.
fn source(source: CoreSource) -> CostSource {
    match source {
        CoreSource::Reported => CostSource::Reported,
        CoreSource::RateCard => CostSource::RateCard,
        CoreSource::Unreported => CostSource::Unreported,
    }
}

/// Describes a set of totals.
pub fn summary(summary: &Summary) -> CostSummary {
    CostSummary {
        runs: i32::try_from(summary.runs).unwrap_or(i32::MAX),
        usd: summary.usd,
        usage: TokenUsage {
            input: i64::try_from(summary.usage.input).unwrap_or(i64::MAX),
            output: i64::try_from(summary.usage.output).unwrap_or(i64::MAX),
            cache_read: i64::try_from(summary.usage.cache_read).unwrap_or(i64::MAX),
            cache_write: i64::try_from(summary.usage.cache_write).unwrap_or(i64::MAX),
        },
        unreported_runs: i32::try_from(summary.unreported_runs).unwrap_or(i32::MAX),
        estimated_runs: i32::try_from(summary.estimated_runs).unwrap_or(i32::MAX),
        confidence: source(summary.confidence()),
        measured_share: summary.measured_share(),
    }
}

/// Describes a resolved window, including how it was reckoned.
pub fn span(span: &Span) -> WindowSpan {
    WindowSpan {
        window: window(span.window),
        label: span.window.label().to_owned(),
        calendar: span.window.is_calendar(),
        start: span.start.map(|at| at.to_string()),
        end: span.end.to_string(),
        zone: span.zone_name().map(ToOwned::to_owned),
        truncated: span.truncated_by_retention(),
    }
}

/// Names a window on the wire.
fn window(window: Window) -> CostWindow {
    match window {
        Window::Today => CostWindow::Today,
        Window::Last24Hours => CostWindow::Last24h,
        Window::Last7Days => CostWindow::Last7d,
        Window::Last30Days => CostWindow::Last30d,
        Window::MonthToDate => CostWindow::MonthToDate,
        Window::Last90Days => CostWindow::Last90d,
        Window::AllTime => CostWindow::AllTime,
    }
}
