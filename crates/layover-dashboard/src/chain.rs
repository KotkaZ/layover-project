//! One chain, whole: `GET /itineraries/{itinerary_id}`.
//!
//! One request rather than four, because the page refreshes it every few seconds while the chain
//! works, and four answers read at four slightly different moments can disagree — a run that has
//! ended in one and is still alive in another — which on a page watching work happen reads as a
//! glitch in the work.

use axum::http::StatusCode;
use jiff::{SignedDuration, Timestamp};
use layover_core::agent::AgentName;
use layover_core::config::Config;
use layover_core::cost::{Span, Window};
use layover_core::diagram::{Layout, Scope, render_svg};
use layover_core::flight::ItineraryId;
use layover_core::handover::RecoveryPolicy;
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::RunRecord;
use layover_http::{GetItineraryPath, Itinerary, ItineraryDetail, Problem, RouteMap, Run};
use layover_store::live::Live;
use layover_store::{HelpFilter, RunFilter};

use crate::api::Dashboard;
use crate::itinerary::{Context, itineraries};

/// One chain's runs and queued work, read at one moment.
pub(crate) struct Gathered {
    /// Its runs, in history or alive now, each once, oldest first.
    pub(crate) records: Vec<RunRecord>,
    /// Those of them alive now.
    alive: Vec<Live>,
    /// Its work waiting to start.
    pending: Vec<Queued>,
    /// The history it was read from.
    span: Span,
}

impl Dashboard {
    /// Everything known about one chain, and its workflow drawn for it alone.
    pub(crate) fn chain(&self, path: &GetItineraryPath) -> Result<ItineraryDetail, Problem> {
        let id = ItineraryId::from(path.itinerary_id.as_str());
        let config = self.config()?;
        let gathered = self.gather(&id, &config)?;
        let itinerary = self.listed(&id, &gathered)?;
        let Gathered {
            records,
            alive,
            pending,
            span: _,
        } = gathered;

        let runs: Vec<Run> = records
            .iter()
            .map(|record| {
                alive
                    .iter()
                    .find(|live| live.run == record.run)
                    .map_or_else(
                        || crate::view::run(record),
                        |live| Self::live_run(live, Some(&config)),
                    )
            })
            .collect();

        // Drawn on its workflow's map, so it can be compared with the workflow at a glance; on the
        // whole factory's when no workflow opened it, or the one that did is no longer declared.
        let pipeline = itinerary
            .pipeline
            .as_deref()
            .map(PipelineName::new)
            .filter(|name| config.pipelines.contains_key(name));
        let drawn = crate::map::chain(&records, &pending, pipeline.as_ref());
        let scope = pipeline.map_or(Scope::Everything, Scope::Pipeline);
        let switched_off = self.switched_off();

        Ok(ItineraryDetail {
            itinerary,
            runs,
            pending: pending
                .iter()
                .map(|queued| crate::view::pending(queued, &switched_off))
                .collect(),
            map: RouteMap {
                mermaid: render_svg(&Layout::scoped(&config, &drawn, &scope)),
                generated_at: Timestamp::now().to_string(),
                config_path: Some(self.state().config_path.display().to_string()),
            },
        })
    }

    /// One chain's runs and queued work, or `404` when nothing has run, is running or is queued in
    /// it.
    pub(crate) fn gather(&self, id: &ItineraryId, config: &Config) -> Result<Gathered, Problem> {
        let span = self.since(id);

        // A run that has just ended can be in history and still have its live record for a moment,
        // because the Tower writes the one before it removes the other. Counted once.
        let alive: Vec<_> = self
            .live_records()
            .into_iter()
            .filter(|live| live.itinerary == *id)
            .collect();
        let mut records: Vec<RunRecord> = self
            .runs(&span, &RunFilter::default())?
            .into_iter()
            .filter(|record| record.itinerary == *id)
            .collect();
        let ended: Vec<_> = records.iter().map(|record| record.run.clone()).collect();
        let alive: Vec<_> = alive
            .into_iter()
            .filter(|live| !ended.contains(&live.run))
            .collect();
        records.extend(
            alive
                .iter()
                .map(|live| Self::live_record(live, Some(config))),
        );
        records.sort_by_key(|record| record.started_at);

        let pending: Vec<Queued> = self
            .state()
            .journal
            .pending()
            .unwrap_or_default()
            .into_iter()
            .filter(|queued| queued.flight.itinerary == *id)
            .collect();
        if records.is_empty() && pending.is_empty() {
            return Err(unknown(id));
        }

        Ok(Gathered {
            records,
            alive,
            pending,
            span,
        })
    }

    /// The chain as `GET /itineraries` lists it, from what [`Self::gather`] read.
    pub(crate) fn listed(
        &self,
        id: &ItineraryId,
        gathered: &Gathered,
    ) -> Result<Itinerary, Problem> {
        let journal = &self.state().journal;
        let stalls = journal.stalls(&gathered.span).unwrap_or_default();
        let help = journal
            .help(&gathered.span, &HelpFilter::default())
            .unwrap_or_default();
        let dismissals = journal.dismissals().unwrap_or_default();
        let off: Vec<PipelineName> = self.switched_off().into_keys().collect();
        let manual = self
            .config()
            .map(|config| manual_agents(&config))
            .unwrap_or_default();

        itineraries(
            &gathered.records,
            Context {
                stalls: &stalls,
                pending: &gathered.pending,
                help: &help,
                ground_stop: self.ground_stop_engaged(),
                switched_off: &off,
                dismissals: &dismissals,
                manual: &manual,
            },
        )
        .into_iter()
        .find(|chain| chain.itinerary_id == id.as_str())
        .ok_or_else(|| unknown(id))
    }

    /// History from when `chain` began: everything retention keeps, narrowed by the time its
    /// identifier carries, so watching a chain reads minutes of history rather than ninety days.
    fn since(&self, chain: &ItineraryId) -> Span {
        let mut span = self.state().history.resolve(Window::AllTime);
        if let Some(began) = chain.minted_at() {
            // A little before, for a clock that moved: a run cannot start before its chain did,
            // but the wall clock it was stamped with can say otherwise.
            let from = began - SignedDuration::from_mins(5);
            span.start = Some(span.start.map_or(from, |kept| kept.max(from)));
        }
        span
    }
}

/// The answer for a chain nothing is known about.
fn unknown(id: &ItineraryId) -> Problem {
    Problem::new(StatusCode::NOT_FOUND, "no such chain").with_detail(format!(
        "nothing has run, is running or is queued in `{}`",
        id.as_str()
    ))
}

/// The agents whose `recovery` is `manual`.
pub(crate) fn manual_agents(config: &Config) -> Vec<AgentName> {
    config
        .agents
        .iter()
        .filter(|(_, agent)| agent.recovery == RecoveryPolicy::Manual)
        .map(|(name, _)| name.clone())
        .collect()
}
