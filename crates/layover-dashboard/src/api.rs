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
use layover_core::diagram::{Layout, Live, Scope, render_svg};
use layover_core::flight::{Flight, ItineraryId, Origin};
use layover_core::pipeline::{FlagError, Pipeline as CorePipeline, PipelineName};
use layover_core::queue::Queued;
use layover_core::run::Outcome;
use layover_http::{
    AgentList, Api, CostBucket, CostReport, CostWindow, EventStream, FlightAccepted, GetCostsQuery,
    GetGraphQuery, GetReportPath, GetRunPath, GroundStop, Health, HelpList, LearningList,
    ListHelpQuery, ListLearningsQuery, ListRunsQuery, PendingList, PipelineList, Problem, Report,
    ReserveState, RouteMap, Run, RunList, RunStatus, SendFlightRequest, Status, StreamRunPath,
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
    /// Help requests, learnings and the queue.
    ///
    /// Shared rather than owned, because in `layover serve` the Tower holds the same journal and
    /// both write to the queue. One handle means one lock.
    pub journal: std::sync::Arc<Journal>,
    /// The Ground Stop file. Its presence means everything is halted.
    pub ground_stop: PathBuf,
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

    /// Whether a Ground Stop is currently engaged.
    ///
    /// Read from disk on every request rather than held in memory, which is the entire reason the
    /// kill switch is a file: it survives a crash and can be set by hand when nothing else is
    /// responding. Reporting a remembered `false` would make the dashboard tell an operator who
    /// had just pulled the handle that nothing was stopped.
    fn ground_stop_engaged(&self) -> bool {
        self.0.ground_stop.exists()
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

    /// Maps each itinerary in `span` to the workflow that started it.
    ///
    /// An itinerary has exactly one pipeline, so this is a lookup rather than a guess. Records
    /// that carry no pipeline — a bare `entry = true` trigger — are simply absent.
    fn pipelines_by_itinerary(
        &self,
        span: &Span,
    ) -> Result<std::collections::HashMap<String, String>, Problem> {
        let runs = self.runs(span, &RunFilter::default())?;

        Ok(runs
            .into_iter()
            .filter_map(|record| {
                record
                    .pipeline
                    .map(|pipeline| (record.itinerary.as_str().to_owned(), pipeline.to_string()))
            })
            .collect())
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
            ground_stop: self.ground_stop_engaged(),
        })
    }

    async fn list_agents(&self) -> Result<AgentList, Problem> {
        Ok(view::agents(&self.config()?))
    }

    async fn list_pipelines(&self) -> Result<PipelineList, Problem> {
        Ok(view::pipelines(&self.config()?))
    }

    async fn get_graph(&self, query: GetGraphQuery) -> Result<RouteMap, Problem> {
        let config = self.config()?;

        let scope = match query.pipeline.as_deref() {
            None => Scope::Everything,
            Some(name) if config.pipelines.contains_key(&name.into()) => {
                Scope::Pipeline(name.into())
            }
            Some(name) => {
                return Err(Problem::new(StatusCode::NOT_FOUND, "no such pipeline")
                    .with_detail(format!("`{name}` is not a pipeline in this factory")));
            }
        };

        let layout = Layout::scoped(&config, &self.live(), &scope);

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
        let scope = RunFilter {
            pipeline: query.pipeline.as_deref().map(PipelineName::new),
            ..RunFilter::default()
        };
        let ledger = self.0.history.ledger_for(&span, &scope).map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "history is unreadable")
                .with_detail(error.to_string())
        })?;

        // The Reserve is metered over its own rolling window, never the one being browsed. They
        // answer different questions — "show me last month" against "what may I still spend
        // today" — and using one ledger for both compared thirty days of spend against a
        // twenty-four hour cap, which is a rail reporting a number that is not true.
        //
        // It is also never narrowed to a workflow. The Reserve caps the factory, so charging one
        // workflow's spend against it would report a rail that does not exist.
        let config = self.config().ok();
        let reserve_hours = config
            .as_ref()
            .map_or(24, |config| config.reserve.window_hours);
        let reserve_span = rolling_back_from(&span, reserve_hours);
        let reserve_ledger = self.0.history.ledger(&reserve_span).map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "history is unreadable")
                .with_detail(error.to_string())
        })?;

        Ok(report(&span, &ledger, &reserve_ledger, config.as_ref()))
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

        // A help request records the itinerary it came from, not the workflow. An itinerary
        // belongs to exactly one pipeline, so the runs already in the window supply the mapping
        // and nothing has to be stored twice.
        let pipelines = self.pipelines_by_itinerary(&span)?;
        let wanted = query.pipeline.as_deref();

        let matching: Vec<_> = requests
            .into_iter()
            .filter(|request| {
                wanted.is_none_or(|name| {
                    pipelines
                        .get(request.itinerary.as_str())
                        .map(String::as_str)
                        == Some(name)
                })
            })
            .collect();

        Ok(HelpList {
            open: i32::try_from(matching.iter().filter(|r| r.is_open()).count())
                .unwrap_or(i32::MAX),
            requests: matching
                .iter()
                .map(|request| {
                    view::help(request, pipelines.get(request.itinerary.as_str()).cloned())
                })
                .collect(),
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

    async fn send_flight(&self, body: SendFlightRequest) -> Result<FlightAccepted, Problem> {
        let config = self.config()?;

        if self.ground_stop_engaged() {
            return Err(
                Problem::new(StatusCode::CONFLICT, "a Ground Stop is engaged")
                    .with_detail("Release it before queueing more work."),
            );
        }

        let (to, pipeline) = resolve_target(&config, &body)?;
        let flags = resolve_flags(&config, pipeline.as_ref(), &body)?;

        let itinerary = ItineraryId::generate();
        let flight = Flight::new(
            itinerary.clone(),
            Origin::Human,
            to.clone(),
            body.body.clone(),
            config.defaults.max_hops,
        );
        let flight_id = flight.id.as_str().to_owned();

        self.0
            .journal
            .queue(Queued::new(flight, pipeline, flags))
            .map_err(|error| {
                Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "the queue is unwritable")
                    .with_detail(error.to_string())
            })?;

        Ok(FlightAccepted {
            flight_id,
            itinerary_id: itinerary.as_str().to_owned(),
            to: to.to_string(),
        })
    }

    async fn list_pending(&self) -> Result<PendingList, Problem> {
        let pending = self.0.journal.pending().map_err(|error| {
            Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "the queue is unreadable")
                .with_detail(error.to_string())
        })?;

        Ok(PendingList {
            pending: pending.iter().map(view::pending).collect(),
            // Null is the honest answer. Showing a queue that looks like it is moving when
            // nothing will move it is the failure this whole surface is meant to avoid.
            dispatched_by: None,
        })
    }

    async fn get_report(&self, path: GetReportPath) -> Result<Report, Problem> {
        let span = self.0.history.resolve(Window::AllTime);
        let found = self
            .0
            .journal
            .report_for(&span, &path.run_id)
            .map_err(|error| {
                Problem::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the journal is unreadable",
                )
                .with_detail(error.to_string())
            })?;

        found.as_ref().map(view::report).ok_or_else(|| {
            Problem::new(StatusCode::NOT_FOUND, "no report for that run").with_detail(format!(
                "`{}` either did not run or wrote nothing",
                path.run_id
            ))
        })
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

/// Works out which agent a trigger is addressed to, and through which pipeline.
///
/// A pipeline is the normal way in: it names the entry agent and declares which flags may be set.
/// A bare `to` is for an agent marked `entry = true`, and accepts no flags.
fn resolve_target(
    config: &layover_core::config::Config,
    body: &SendFlightRequest,
) -> Result<(layover_core::agent::AgentName, Option<PipelineName>), Problem> {
    if let Some(name) = &body.pipeline {
        let key: PipelineName = name.as_str().into();
        let pipeline = config.pipelines.get(&key).ok_or_else(|| {
            Problem::new(StatusCode::BAD_REQUEST, "no such pipeline")
                .with_detail(format!("`{name}` is not a pipeline in this factory"))
        })?;
        return Ok((pipeline.entry.clone(), Some(key)));
    }

    let Some(name) = &body.to else {
        return Err(
            Problem::new(StatusCode::BAD_REQUEST, "give either a pipeline or a to").with_detail(
                "A pipeline is the normal way in; `to` is for an agent marked `entry = true`.",
            ),
        );
    };

    let agent: layover_core::agent::AgentName = name.as_str().into();
    match config.agents.get(&agent) {
        Some(found) if found.entry => Ok((agent, None)),
        Some(_) => Err(
            Problem::new(StatusCode::BAD_REQUEST, "that agent is not an entry point").with_detail(
                format!(
                    "`{name}` exists but is not marked `entry = true`, so work may not be sent \
                 straight to it"
                ),
            ),
        ),
        None => Err(Problem::new(StatusCode::BAD_REQUEST, "no such agent")
            .with_detail(format!("`{name}` is not an agent in this factory"))),
    }
}

/// Resolves the flags for a trigger, filling in the pipeline''s declared defaults.
///
/// A flag the pipeline does not declare is refused rather than ignored: silently dropping it
/// would let a typo change nothing while appearing to work.
fn resolve_flags(
    config: &layover_core::config::Config,
    pipeline: Option<&PipelineName>,
    body: &SendFlightRequest,
) -> Result<std::collections::BTreeMap<String, bool>, Problem> {
    let asked = body.flags.clone().unwrap_or_default();

    let Some(name) = pipeline else {
        if asked.is_empty() {
            return Ok(std::collections::BTreeMap::new());
        }
        return Err(
            Problem::new(StatusCode::BAD_REQUEST, "flags need a pipeline")
                .with_detail("Only a pipeline declares flags, so a bare `to` accepts none."),
        );
    };

    let declared: &CorePipeline = config
        .pipelines
        .get(name)
        .ok_or_else(|| Problem::new(StatusCode::BAD_REQUEST, "no such pipeline"))?;

    declared.flags_for_run(&asked).map_or_else(
        |error| match error {
            FlagError::Undeclared { flag } => Err(Problem::new(
                StatusCode::BAD_REQUEST,
                "that pipeline does not declare that flag",
            )
            .with_detail(format!(
                "`{flag}` is not declared by `{name}`. A flag that is silently ignored is a typo \
                 that changes nothing while appearing to work"
            ))),
        },
        |flags| Ok(flags.iter().map(|(k, v)| (k.to_owned(), v)).collect()),
    )
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
fn report(
    span: &Span,
    ledger: &Ledger,
    reserve_ledger: &Ledger,
    config: Option<&Config>,
) -> CostReport {
    let reserve = config.map(|config| &config.reserve);
    let reserve_spent = reserve_ledger.total().usd;

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
        by_pipeline: ranked(
            ledger
                .by_pipeline()
                .into_iter()
                .map(|(name, summary)| (name.to_string(), summary))
                .collect(),
        ),
        reserve: ReserveState {
            cap_usd: reserve
                .map(|reserve| reserve.fuel_usd)
                .filter(|cap| *cap > 0.0),
            spent_usd: reserve_spent,
            remaining_usd: reserve
                .map(|reserve| reserve.fuel_usd)
                .filter(|cap| *cap > 0.0)
                .map(|cap| (cap - reserve_spent).max(0.0)),
            window_hours: reserve.map_or(24, |reserve| {
                i64::try_from(reserve.window_hours).unwrap_or(24)
            }),
            exhausted: reserve
                .is_some_and(|reserve| reserve.fuel_usd > 0.0 && reserve_spent >= reserve.fuel_usd),
        },
    }
}

/// Narrows `span` to the `hours` immediately before it ends.
///
/// The Reserve is configured in hours and has no matching [`Window`] variant, so its period is
/// derived rather than resolved. Only the total is read from the resulting ledger; the span itself
/// is never reported, which is why carrying the browsing window's label here is harmless.
fn rolling_back_from(span: &Span, hours: u64) -> Span {
    let hours = i64::try_from(hours).unwrap_or(i64::MAX).min(1_000_000);
    let start = span
        .end
        .checked_sub(jiff::SignedDuration::from_hours(hours))
        .unwrap_or(Timestamp::MIN);

    Span {
        start: Some(start),
        ..span.clone()
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
