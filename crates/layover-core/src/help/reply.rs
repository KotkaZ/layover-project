//! Answering a help request by continuing the work it stopped.
//!
//! An agent that files a fatal request ends its run, and with it the chain: nothing will run again
//! until a person starts something. Marking the request resolved does not do that — there is no
//! "next run" to notice — so a reply is the way back in. It starts a **new chain** to the agent
//! that asked, with the person's answer, and is priced like a resumed layover: a fresh budget,
//! because the chain that asked is over, carrying everything about *how* the work was asked for.
//!
//! # What a reply carries
//!
//! The asking chain's workflow, flags and routes, exactly — never the pipeline's defaults. An
//! operator who triggered a run allowed to open a pull request, then answered its question, must
//! not get back a run that may not: that is the silent downgrade a reply exists to avoid. A
//! request filed before Layover recorded its chain does not know them, so the operator has to say,
//! and is refused until they do.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::Config;
use crate::flight::{Flight, Origin};
use crate::help::HelpRequest;
use crate::pipeline::{FlagError, PipelineName};
use crate::queue::Queued;

/// How every reply begins, so the agent can recognise one.
pub const HEADER: &str = "In reply to your help request";

/// Why a reply cannot continue the work.
#[derive(Debug, thiserror::Error)]
pub enum Refused {
    /// There was nothing to say.
    #[error("a reply needs something to say")]
    Empty,
    /// The request predates Layover recording its chain's flags, and the operator did not choose
    /// them.
    #[error(
        "this request was filed before Layover recorded its chain's flags, so `{pipeline}` cannot \
         be continued as it was asked for without them. Say which of {declared} it had."
    )]
    FlagsNotRecorded {
        /// The workflow being continued.
        pipeline: PipelineName,
        /// The flags it declares, for the message.
        declared: String,
    },
    /// Flags were chosen for a request that already records its chain's.
    #[error(
        "this request records the flags its chain ran with, and a reply carries those. To change \
         them, continue the chain instead"
    )]
    FlagsAlreadyRecorded,
    /// Flags were chosen for a chain no workflow opened, which declares none.
    #[error("the chain that asked belongs to no workflow, so it has no flags to set")]
    FlagsWithoutPipeline,
    /// A chosen flag is not one the workflow declares.
    #[error(transparent)]
    Flag(#[from] FlagError),
}

/// What the agent is told: a fixed first line naming the request it answers, then the person's
/// words, verbatim.
///
/// `asked` is every open request the reply answers, all from one run.
#[must_use]
pub fn body(asked: &[HelpRequest], text: &str) -> String {
    let run = asked
        .first()
        .map(|request| request.run.as_str())
        .unwrap_or_default();
    let summaries = asked
        .iter()
        .map(|request| request.summary.trim())
        .collect::<Vec<_>>()
        .join("; ");
    format!("{HEADER} {run} ({summaries})\n\n{text}")
}

/// The flight that continues the asking chain's work with the person's answer.
///
/// `chosen` is flags the operator set; they are used only for a request that does not record its
/// chain's. `pipeline` is the workflow the asking chain belonged to as far as anything else
/// records it, for such a request.
///
/// # Errors
///
/// Returns [`Refused`] when there is nothing to say, when flags are needed and not given or given
/// and not needed, or when a chosen flag is not declared.
pub fn continuation(
    config: &Config,
    asked: &[HelpRequest],
    text: &str,
    chosen: Option<&BTreeMap<String, bool>>,
    pipeline: Option<PipelineName>,
) -> Result<Queued, Refused> {
    let Some(first) = asked.first() else {
        return Err(Refused::Empty);
    };
    if text.trim().is_empty() {
        return Err(Refused::Empty);
    }

    let (pipeline, flags, within) = if let Some(scope) = &first.scope {
        if chosen.is_some_and(|chosen| !chosen.is_empty()) {
            return Err(Refused::FlagsAlreadyRecorded);
        }
        (
            scope.pipeline.clone(),
            carried(config, scope.pipeline.as_ref(), &first.flags),
            scope.within.clone(),
        )
    } else {
        let flags = said(config, pipeline.as_ref(), chosen)?;
        (pipeline, flags, BTreeSet::new())
    };

    // A person sent it: that is who the agent is told wrote it. A new chain with a fresh budget,
    // because the one that asked is over.
    let flight = Flight::new(
        crate::flight::ItineraryId::generate(),
        Origin::Human,
        first.agent.clone(),
        body(asked, text),
        config.defaults.max_hops,
    );

    // The continuation is the same piece of work, so it keeps its name and runs its agents as the
    // chain that asked was asked to.
    Ok(Queued::new(flight, pipeline, flags)
        .narrowed_by(within)
        .continuing(first.itinerary.clone())
        .choosing(first.chosen.clone()))
}

/// The flags a request recorded, within what its workflow declares now, as a resumed layover's are:
/// a flag since removed is dropped, and one since added takes its default.
fn carried(
    config: &Config,
    pipeline: Option<&PipelineName>,
    recorded: &BTreeMap<String, bool>,
) -> BTreeMap<String, bool> {
    pipeline
        .and_then(|name| config.pipelines.get(name))
        .map_or_else(
            || recorded.clone(),
            |declared| declared.flags_carrying(recorded).to_map(),
        )
}

/// The flags the person replying says a chain had, for a request that did not record them.
fn said(
    config: &Config,
    pipeline: Option<&PipelineName>,
    chosen: Option<&BTreeMap<String, bool>>,
) -> Result<BTreeMap<String, bool>, Refused> {
    let declared = pipeline.and_then(|name| config.pipelines.get(name));
    match (declared, chosen) {
        (Some(declared), Some(chosen)) => Ok(declared.flags_for_run(chosen)?.to_map()),
        (Some(declared), None) if declared.flags.is_empty() => Ok(BTreeMap::new()),
        (Some(declared), None) => Err(Refused::FlagsNotRecorded {
            pipeline: pipeline.cloned().unwrap_or_else(|| PipelineName::new("")),
            declared: declared
                .flags
                .keys()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", "),
        }),
        (None, Some(chosen)) if !chosen.is_empty() => Err(Refused::FlagsWithoutPipeline),
        (None, _) => Ok(BTreeMap::new()),
    }
}
