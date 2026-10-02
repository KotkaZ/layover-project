//! One chain, whole: `GET /itineraries/{itinerary_id}`.
//!
//! One request rather than four, because the page refreshes it every few seconds while the chain
//! works, and four answers read at four slightly different moments can disagree — a run that has
//! ended in one and is still alive in another — which on a page watching work happen reads as a
//! glitch in the work.

use axum::http::StatusCode;
use jiff::{SignedDuration, Timestamp};
use layover_core::cost::{Span, Window};
use layover_core::diagram::{Layout, Scope, render_svg};
use layover_core::flight::ItineraryId;
use layover_core::pipeline::PipelineName;
use layover_core::queue::Queued;
use layover_core::run::RunRecord;
use layover_http::{GetItineraryPath, ItineraryDetail, Problem, RouteMap, Run};
use layover_store::{HelpFilter, RunFilter};

use crate::api::Dashboard;
use crate::itinerary::{Context, itineraries};

impl Dashboard {
    /// Everything known about one chain, and its workflow drawn for it alone.
    pub(crate) fn chain(&self, path: &GetItineraryPath) -> Result<ItineraryDetail, Problem> {
        let id = ItineraryId::from(path.itinerary_id.as_str());
        let unknown = || {
            Problem::new(StatusCode::NOT_FOUND, "no such chain").with_detail(format!(
                "nothing has run, is running or is queued in `{}`",
                path.itinerary_id
            ))
        };
        let config = self.config()?;
        let span = self.since(&id);

        // A run that has just ended can be in history and still have its live record for a moment,
        // because the Tower writes the one before it removes the other. Counted once.
        let alive: Vec<_> = self
            .live_records()
            .into_iter()
            .filter(|live| live.itinerary == id)
            .collect();
        let mut records: Vec<RunRecord> = self
            .runs(&span, &RunFilter::default())?
            .into_iter()
            .filter(|record| record.itinerary == id)
            .collect();
        let ended: Vec<_> = records.iter().map(|record| record.run.clone()).collect();
        let alive: Vec<_> = alive
            .into_iter()
            .filter(|live| !ended.contains(&live.run))
            .collect();
        records.extend(
            alive
                .iter()
                .map(|live| Self::live_record(live, Some(&config))),
        );
        records.sort_by_key(|record| record.started_at);

        let pending: Vec<Queued> = self
            .state()
            .journal
            .pending()
            .unwrap_or_default()
            .into_iter()
            .filter(|queued| queued.flight.itinerary == id)
            .collect();
        if records.is_empty() && pending.is_empty() {
            return Err(unknown());
        }

        let journal = &self.state().journal;
        let stalls = journal.stalls(&span).unwrap_or_default();
        let help = journal
            .help(&span, &HelpFilter::default())
            .unwrap_or_default();
        let itinerary = itineraries(
            &records,
            Context {
                stalls: &stalls,
                pending: &pending,
                help: &help,
                ground_stop: self.ground_stop_engaged(),
            },
        )
        .into_iter()
        .find(|chain| chain.itinerary_id == path.itinerary_id)
        .ok_or_else(unknown)?;

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

        Ok(ItineraryDetail {
            itinerary,
            runs,
            pending: pending.iter().map(crate::view::pending).collect(),
            map: RouteMap {
                mermaid: render_svg(&Layout::scoped(&config, &drawn, &scope)),
                generated_at: Timestamp::now().to_string(),
                config_path: Some(self.state().config_path.display().to_string()),
            },
        })
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
