//! The HTTP API, backed by the configuration on disk and the run history.
//!
//! Every handler is synchronous underneath: reading a few kilobytes of TOML and a handful of
//! small files is faster than the scheduling it would take to move the work off-thread, and this
//! is a single-user server on loopback. The `Api` trait is async because the Tower's eventual
//! implementation will be, so the handlers here satisfy it without awaiting anything.
#![allow(clippy::unused_async_trait_impl)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use jiff::Timestamp;
use layover_core::config::Config;
use layover_core::cost::{Ledger, Span, Window};
use layover_core::diagram::{Layout, Live, render_svg};
use layover_core::run::Outcome;
use layover_http::{
    AgentList, Api, CostBucket, CostReport, CostWindow, EventStream, FlightAccepted, GetCostsQuery,
    GetRunPath, GroundStop, Health, HelpList, LearningList, ListHelpQuery, ListLearningsQuery,
    ListRunsQuery, PipelineList, Problem, ReserveState, RouteMap, Run, RunList, RunStatus,
    SendFlightRequest, Status, StreamRunPath,
};
use layover_store::{HelpFilter, History, Journal, RunFilter};

use crate::view;

/// Everything the dashboard reads from.
#[derive(Debug, Clone)]
pub struct DashboardState {
    /// Where the factory definition lives.
    pub config_path: PathBuf,
    /// The run history.
    pub history: History,
    /// Help requests and learnings.
    pub journal: Journal,
}

/// The dashboard's implementation of the Tower API.
///
/// Reads the configuration from disk on every request rather than caching it. That is the
/// difference between a dashboard and a snapshot: edit `layover.toml`, reload the page, and the
/// route map changes with it. A factory definition is a few kilobytes of TOML, so the cost of
/// re-reading it is not worth the surprise of a stale diagram.
#[derive(Debug, Clone)]
pub struct Dashboard(Arc<DashboardState>);

impl Dashboard {
    /// Creates a dashboard over a configuration file and a history directory.
    #[must_use]
    pub fn new(state: DashboardState) -> Self {
        Self(Arc::new(state))
    }

    /// The state this dashboard reads.
    #[must_use]
    pub fn state(&self) -> &DashboardState {
        &self.0
    }

    /// Loads the factory definition as it is on disk right now.
    fn config(&self) -> Result<Config, Problem> {
        Config::load(&self.0.config_path).map_err(|error| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the factory definition could not be read",
            )
            .with_detail(error.to_string())
        })
    }

    /// Resolves a window against the history's zone.
    fn span(&self, window: Option<CostWindow>) -> Span {
        self.0.history.resolve(to_window(window))
    }

    /// Reads runs, turning a store failure into a problem rather than a panic.
    fn runs(
        &self,
        span: &Span,
        filter: &RunFilter,
    ) -> Result<Vec<layover_core::RunRecord>, Problem> {
        self.0.history.runs(span, filter).map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "history is unreadable")
                .with_detail(error.to_string())
        })
    }

    /// What the factory is doing right now, for colouring the route map.
    ///
    /// Derived from history rather than from a live supervisor, because there is not one yet. A
    /// record left in `running` is either genuinely live or was interrupted, and until the Tower
    /// exists those are indistinguishable from here — which is itself worth seeing.
    fn live(&self) -> Live {
        let span = self.0.history.resolve(Window::Last24Hours);
        let mut live = Live::default();

        let Ok(records) = self.0.history.runs(&span, &RunFilter::default()) else {
            return live;
        };

        // Oldest first so that a later run of the same agent overwrites an earlier one: the most
        // recent state is the one worth showing.
        for record in records.into_iter().rev() {
            match record.outcome {
                Outcome::Running => {
                    live.activity
                        .insert(record.agent, layover_core::diagram::Activity::Running);
                }
                outcome if outcome.is_failure() => {
                    live.activity
                        .insert(record.agent, layover_core::diagram::Activity::Failed);
                }
                _ => {
                    live.activity.remove(&record.agent);
                }
            }
        }

        live
    }
}

impl Api for Dashboard {
    async fn get_health(&self) -> Result<Health, Problem> {
        Ok(Health {
            status: Status::Ok,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ground_stop: false,
        })
    }

    async fn list_agents(&self) -> Result<AgentList, Problem> {
        Ok(view::agents(&self.config()?))
    }

    async fn list_pipelines(&self) -> Result<PipelineList, Problem> {
        Ok(view::pipelines(&self.config()?))
    }

    async fn get_graph(&self) -> Result<RouteMap, Problem> {
        let config = self.config()?;
        let layout = Layout::build(&config, &self.live());

        Ok(RouteMap {
            mermaid: render_svg(&layout),
            generated_at: Timestamp::now().to_string(),
            config_path: Some(self.0.config_path.display().to_string()),
        })
    }

    async fn list_runs(&self, query: ListRunsQuery) -> Result<RunList, Problem> {
        let span = self.span(query.window);
        let filter = RunFilter {
            agent: query.agent.as_deref().map(Into::into),
            pipeline: query.pipeline.as_deref().map(Into::into),
            outcome: query.status.map(from_status),
            limit: query.limit.and_then(|n| usize::try_from(n).ok()),
        };

        let mut records = self.runs(&span, &filter)?;
        if let Some(itinerary) = &query.itinerary_id {
            records.retain(|record| record.itinerary.as_str() == itinerary);
        }

        Ok(RunList {
            runs: records.iter().map(view::run).collect(),
        })
    }

    async fn get_run(&self, path: GetRunPath) -> Result<Run, Problem> {
        let span = self.0.history.resolve(Window::AllTime);
        let found = self
            .runs(&span, &RunFilter::default())?
            .into_iter()
            .find(|record| record.run.as_str() == path.run_id);

        found.as_ref().map(view::run).ok_or_else(|| {
            Problem::new(StatusCode::NOT_FOUND, "no such run")
                .with_detail(format!("`{}` is not in history", path.run_id))
        })
    }

    async fn get_costs(&self, query: GetCostsQuery) -> Result<CostReport, Problem> {
        let span = self.span(query.window.or(Some(CostWindow::Last30d)));
        let ledger = self.0.history.ledger(&span).map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "history is unreadable")
                .with_detail(error.to_string())
        })?;

        Ok(report(&span, &ledger, self.config().ok().as_ref()))
    }

    async fn list_help(&self, query: ListHelpQuery) -> Result<HelpList, Problem> {
        let span = self.span(query.window.or(Some(CostWindow::Last30d)));
        let filter = HelpFilter {
            agent: query.agent.as_deref().map(Into::into),
            blocker: query.blocker.map(view::blocker_from),
            open_only: query.open.unwrap_or(false),
            fatal_only: false,
        };

        let requests = self.0.journal.help(&span, &filter).map_err(|error| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the journal is unreadable",
            )
            .with_detail(error.to_string())
        })?;

        Ok(HelpList {
            open: i32::try_from(requests.iter().filter(|r| r.is_open()).count())
                .unwrap_or(i32::MAX),
            requests: requests.iter().map(view::help).collect(),
        })
    }

    async fn list_learnings(&self, query: ListLearningsQuery) -> Result<LearningList, Problem> {
        let learnings = self.0.journal.learnings().map_err(|error| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the journal is unreadable",
            )
            .with_detail(error.to_string())
        })?;

        let wanted = query.state.map(view::learning_state_from);
        let matching: Vec<&layover_core::learning::Learning> = learnings
            .all()
            .filter(|learning| {
                query
                    .agent
                    .as_deref()
                    .is_none_or(|agent| learning.agent.as_str() == agent)
                    && wanted.is_none_or(|state| learning.state == state)
            })
            .collect();

        Ok(LearningList {
            active: i32::try_from(matching.iter().filter(|l| l.is_active()).count())
                .unwrap_or(i32::MAX),
            learnings: matching.into_iter().map(view::learning).collect(),
        })
    }

    async fn send_flight(&self, _: SendFlightRequest) -> Result<FlightAccepted, Problem> {
        Err(not_supervised("sending a flight"))
    }

    async fn stream_run(&self, _: StreamRunPath) -> Result<EventStream, Problem> {
        Err(not_supervised("streaming a run"))
    }

    async fn engage_ground_stop(&self) -> Result<GroundStop, Problem> {
        Err(not_supervised("engaging a Ground Stop"))
    }

    async fn release_ground_stop(&self) -> Result<GroundStop, Problem> {
        Err(not_supervised("releasing a Ground Stop"))
    }
}

/// Refuses an operation that needs a supervisor, and says so.
///
/// `501` rather than a plausible-looking success. A control that silently does nothing is worse
/// than a control that is not there, because it is trusted once and then relied on.
fn not_supervised(what: &str) -> Problem {
    Problem::new(StatusCode::NOT_IMPLEMENTED, format!("{what} needs a Tower")).with_detail(
        "The dashboard reads history and configuration. Starting, stopping and steering work \
             requires the supervisor, which is not part of this release.",
    )
}

/// Turns the API's window into the domain's.
fn to_window(window: Option<CostWindow>) -> Window {
    match window {
        Some(CostWindow::Today) => Window::Today,
        Some(CostWindow::Last24h) | None => Window::Last24Hours,
        Some(CostWindow::Last7d) => Window::Last7Days,
        Some(CostWindow::Last30d) => Window::Last30Days,
        Some(CostWindow::MonthToDate) => Window::MonthToDate,
        Some(CostWindow::Last90d) => Window::Last90Days,
        Some(CostWindow::AllTime) => Window::AllTime,
    }
}

/// Turns the API's run status into the domain's outcome.
fn from_status(status: RunStatus) -> Outcome {
    match status {
        RunStatus::Running => Outcome::Running,
        RunStatus::Succeeded => Outcome::Succeeded,
        RunStatus::Failed => Outcome::Failed,
        RunStatus::TimedOut => Outcome::TimedOut,
        RunStatus::Halted => Outcome::Halted,
        RunStatus::Interrupted => Outcome::Interrupted,
    }
}

/// Builds the cost report for a window.
fn report(span: &Span, ledger: &Ledger, config: Option<&Config>) -> CostReport {
    let reserve = config.map(|config| &config.reserve);

    CostReport {
        span: view::span(span),
        total: view::summary(&ledger.total()),
        by_agent: ledger
            .top_agents(usize::MAX)
            .into_iter()
            .map(|(name, summary)| CostBucket {
                name: name.to_string(),
                summary: view::summary(&summary),
            })
            .collect(),
        by_model: ranked(ledger.by_model()),
        reserve: ReserveState {
            cap_usd: reserve
                .map(|reserve| reserve.fuel_usd)
                .filter(|cap| *cap > 0.0),
            spent_usd: ledger.total().usd,
            remaining_usd: reserve
                .map(|reserve| reserve.fuel_usd)
                .filter(|cap| *cap > 0.0)
                .map(|cap| (cap - ledger.total().usd).max(0.0)),
            window_hours: reserve.map_or(24, |reserve| {
                i64::try_from(reserve.window_hours).unwrap_or(24)
            }),
            exhausted: reserve.is_some_and(|reserve| {
                reserve.fuel_usd > 0.0 && ledger.total().usd >= reserve.fuel_usd
            }),
        },
    }
}

/// Orders a breakdown most expensive first.
fn ranked(
    buckets: std::collections::BTreeMap<String, layover_core::cost::Summary>,
) -> Vec<CostBucket> {
    let mut ordered: Vec<(String, layover_core::cost::Summary)> = buckets.into_iter().collect();
    ordered.sort_by(|(left_name, left), (right_name, right)| {
        right
            .usd
            .total_cmp(&left.usd)
            .then_with(|| left_name.cmp(right_name))
    });

    ordered
        .into_iter()
        .map(|(name, summary)| CostBucket {
            name,
            summary: view::summary(&summary),
        })
        .collect()
}
