//! What is running now, read from the live records the Tower keeps while a run is going.
//!
//! History holds a run only once it has ended, so a dashboard reading only history could never
//! show what is happening — it said "running" only for a record nothing writes. The live records
//! are the truth, and they are on disk: a dashboard can read them whichever process started the
//! runs, a `serve` beside it or one in another terminal.

use std::path::PathBuf;

use axum::http::StatusCode;
use jiff::Timestamp;
use layover_core::RunId;
use layover_core::config::Config;
use layover_core::cost::Window;
use layover_core::model::ModelChoice;
use layover_core::run::RunRecord;
use layover_http::{Problem, Run};
use layover_store::RunFilter;
use layover_store::live::{Ledger, Live};

use crate::api::Dashboard;
use crate::stream::Source;

impl Dashboard {
    /// The factory's state directory, beside its configuration.
    fn state_dir(&self) -> PathBuf {
        self.state()
            .config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".layover")
    }

    /// The live records, read without creating anything.
    pub(crate) fn ledger(&self) -> Ledger {
        Ledger::at(self.state_dir().join("state").join("runs"))
    }

    /// Every run alive now, newest first.
    pub(crate) fn live_records(&self) -> Vec<Live> {
        let mut live = self.ledger().live().unwrap_or_default();
        live.reverse();
        live
    }

    /// A live run as history would hold it if it had ended: still `running`, not finished, with
    /// the workflow, flags and link to an earlier chain that its queued work carried.
    pub(crate) fn live_record(live: &Live, config: Option<&Config>) -> RunRecord {
        let mut record = RunRecord::started(
            live.run.clone(),
            live.itinerary.clone(),
            live.agent.clone(),
            live.started_at,
        );
        if let Some(queued) = &live.queued {
            record.pipeline.clone_from(&queued.pipeline);
            if queued.pipeline.is_some() {
                record.flags.clone_from(&queued.flags);
            }
            record.continues.clone_from(&queued.continues);
        }
        record.sent_by.clone_from(&live.sent_by);
        record.model = config.and_then(|config| ModelChoice::of(config, &live.agent).model);
        record
    }

    /// A live run as the API shows a run: `running`, how long so far, and nothing billed yet.
    pub(crate) fn live_run(live: &Live, config: Option<&Config>) -> Run {
        let record = Self::live_record(live, config);
        let mut run = crate::view::run(&record);
        run.duration_sec = Some(Timestamp::now().as_second() - live.started_at.as_second());
        run
    }

    /// Where `run_id`'s output is, whether it is alive or over.
    pub(crate) fn source(&self, run_id: &str) -> Result<Source, Problem> {
        let unknown = || {
            Problem::new(StatusCode::NOT_FOUND, "no such run")
                .with_detail(format!("`{run_id}` is neither running nor in history"))
        };
        // An identifier names a file below, so only one that could be a run's is looked for.
        let plausible = !run_id.is_empty()
            && run_id.len() <= 64
            && run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !plausible {
            return Err(unknown());
        }

        let run = RunId::from(run_id);
        let ledger = self.ledger();
        let history = self.state().history.clone();

        if let Some(live) = ledger.find(&run) {
            return Ok(Source {
                transcript: live.hangar.join(layover_store::hangar::TRANSCRIPT),
                run,
                live: Some(ledger),
                history,
            });
        }

        let span = history.resolve(Window::AllTime);
        let record = history
            .runs(&span, &RunFilter::default())
            .ok()
            .and_then(|records| records.into_iter().find(|record| record.run == run))
            .ok_or_else(unknown)?;
        let hangar = layover_store::hangar::run_dir(
            &self.state_dir().join("hangars"),
            &record.agent,
            &record.run,
        );

        Ok(Source {
            transcript: hangar.join(layover_store::hangar::TRANSCRIPT),
            run,
            live: None,
            history,
        })
    }
}
