//! Switching a workflow off and on, and what the rest of the dashboard asks about it.
//!
//! The switch lives in the journal, where the Tower reads it on every pass — see
//! [`layover_store::SwitchedOff`]. This is the dashboard's side: flipping it, refusing a trigger
//! for a workflow that is off, and telling every view which workflows are.

use std::collections::BTreeMap;

use axum::http::StatusCode;
use jiff::Timestamp;
use layover_core::pipeline::PipelineName;
use layover_http::{PipelineSwitch, Problem};

use crate::api::Dashboard;

/// Each switched-off workflow, and when it was switched off where that is known.
pub(crate) type Off = BTreeMap<PipelineName, Option<Timestamp>>;

impl Dashboard {
    /// Every workflow that is switched off.
    ///
    /// For showing, so an unreadable file shows nothing as off — the Tower, reading the same file,
    /// holds nothing back either. Refusing a trigger asks [`Self::refuse_if_off`], which does not
    /// guess.
    pub(crate) fn switched_off(&self) -> Off {
        self.state()
            .journal
            .switched_off()
            .unwrap_or_default()
            .into_iter()
            .map(|off| (off.pipeline, off.since))
            .collect()
    }

    /// Switches `name` off or on.
    pub(crate) fn switch(&self, name: &str, off: bool) -> Result<PipelineSwitch, Problem> {
        let config = self.config()?;
        let pipeline = PipelineName::new(name);
        if !config.pipelines.contains_key(&pipeline) {
            return Err(Problem::new(StatusCode::NOT_FOUND, "no such pipeline")
                .with_detail(format!("`{name}` is not a pipeline in this factory")));
        }

        let journal = &self.state().journal;
        let failed = |error: layover_store::StoreError| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not change the switch",
            )
            .with_detail(format!(
                "{error}. Add or remove `{{\"pipeline\":\"{name}\"}}` in {} by hand; the Tower \
                 reads it on every pass.",
                journal.root().join("switched-off.jsonl").display()
            ))
        };

        if off {
            let now = journal
                .switch_off(&pipeline, Timestamp::now())
                .map_err(failed)?;
            Ok(PipelineSwitch {
                pipeline: name.to_owned(),
                switched_off: true,
                since: now.since.map(|at| at.to_string()),
            })
        } else {
            journal.switch_on(&pipeline).map_err(failed)?;
            Ok(PipelineSwitch {
                pipeline: name.to_owned(),
                switched_off: false,
                since: None,
            })
        }
    }

    /// Refuses a trigger for a workflow that is switched off.
    ///
    /// A switch that let a trigger through would not be off; one that queued it to wait would
    /// leave a person thinking their run was on its way. Refused, they are told why at once.
    pub(crate) fn refuse_if_off(&self, pipeline: Option<&PipelineName>) -> Result<(), Problem> {
        let Some(pipeline) = pipeline else {
            return Ok(());
        };
        let off = self.state().journal.switched_off().map_err(|error| {
            Problem::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not tell whether that workflow is switched off",
            )
            .with_detail(error.to_string())
        })?;
        if off.iter().any(|off| off.pipeline == *pipeline) {
            return Err(
                Problem::new(StatusCode::CONFLICT, "that workflow is switched off")
                    .with_detail(format!("Switch `{pipeline}` back on before triggering it.")),
            );
        }
        Ok(())
    }
}
