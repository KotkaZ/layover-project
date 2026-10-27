//! Dismissing a stalled or failed chain from the dashboard, and undoing it.
//!
//! What a dismissal is, and how long it is kept, is the store's — see [`layover_store::Dismissal`].
//! Which chains it applies to is decided where chains are built. This is the dashboard's side:
//! refusing a chain there is nothing to dismiss on, naming who did it, and answering with the
//! chain as it now reads.

use axum::http::StatusCode;
use jiff::Timestamp;
use layover_core::flight::ItineraryId;
use layover_http::{DismissItineraryRequest, Itinerary, ItineraryState, Problem};
use layover_store::Dismissal;

use crate::api::Dashboard;
use crate::itinerary::ended_on;

impl Dashboard {
    /// Dismisses `id`, which must be stalled or failed.
    pub(crate) fn dismiss(
        &self,
        id: &str,
        request: &DismissItineraryRequest,
    ) -> Result<Itinerary, Problem> {
        let id = ItineraryId::from(id);
        let config = self.config()?;
        let gathered = self.gather(&id, &config)?;
        let chain = self.listed(&id, &gathered)?;

        // A second press is a success that changes nothing, like resolving a help request twice.
        if chain.dismissed.is_some() {
            return Ok(chain);
        }
        if let Some(why) = not_dismissable(chain.state) {
            return Err(Problem::new(StatusCode::CONFLICT, why.0).with_detail(why.1));
        }

        let runs: Vec<_> = gathered.records.iter().collect();
        let Some(ended) = ended_on(&runs) else {
            return Err(
                Problem::new(StatusCode::CONFLICT, "nothing in it has ended yet").with_detail(
                    "A dismissal is of the run a chain ended on; wait for one to end.",
                ),
            );
        };

        self.state()
            .journal
            .dismiss(Dismissal {
                itinerary: id.clone(),
                run: ended.run.clone(),
                by: crate::reply::who(request.by.as_deref()),
                at: Timestamp::now(),
                note: request
                    .note
                    .as_deref()
                    .map(str::trim)
                    .filter(|note| !note.is_empty())
                    .map(str::to_owned),
            })
            .map_err(|error| unwritable(&error))?;

        self.listed(&id, &self.gather(&id, &config)?)
    }

    /// Undoes `id`'s dismissal, if it has one.
    pub(crate) fn undismiss(&self, id: &str) -> Result<Itinerary, Problem> {
        let id = ItineraryId::from(id);
        let config = self.config()?;
        // Asked first, so a chain nobody has heard of is a `404` rather than a success.
        self.gather(&id, &config)?;

        self.state()
            .journal
            .undismiss(&id)
            .map_err(|error| unwritable(&error))?;

        self.listed(&id, &self.gather(&id, &config)?)
    }
}

/// Why a chain in `state` cannot be dismissed, as a title and a detail, or `None` when it can.
fn not_dismissable(state: ItineraryState) -> Option<(&'static str, &'static str)> {
    match state {
        ItineraryState::Stalled | ItineraryState::Failed => None,
        ItineraryState::AwaitingHuman => Some((
            "it is waiting for an answer",
            "Reply to its help request to continue the work, or resolve it to say it is dealt with.",
        )),
        ItineraryState::Working => Some((
            "it is still working",
            "Only a stalled or failed chain can be dismissed. Cancel its queued work, or engage a \
             Ground Stop, to stop it.",
        )),
        ItineraryState::Finished => Some((
            "it finished",
            "Only a stalled or failed chain can be dismissed, and this one needs nobody.",
        )),
        ItineraryState::Halted => Some((
            "it is halted",
            "Only a stalled or failed chain can be dismissed.",
        )),
    }
}

/// The answer for a dismissal that could not be written.
fn unwritable(error: &layover_store::StoreError) -> Problem {
    Problem::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the journal is unwritable",
    )
    .with_detail(error.to_string())
}
