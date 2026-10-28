//! Which dismissal, if any, stands for a chain. See the module above for what a dismissal is.

use layover_core::run::RunRecord;
use layover_http::ItineraryState;
use layover_store::Dismissal;

/// The dismissal that stands for a chain: one of the run it ended on, while it is stalled or failed.
///
/// Only then. A chain that has since been continued no longer needs one, and one that ended on a
/// later run has a failure nobody has looked at.
pub(super) fn dismissed(
    id: &str,
    state: ItineraryState,
    ended: Option<&RunRecord>,
    dismissals: &[Dismissal],
) -> Option<layover_http::Dismissal> {
    if !matches!(state, ItineraryState::Stalled | ItineraryState::Failed) {
        return None;
    }
    let ended = ended?;
    dismissals
        .iter()
        .find(|dismissal| dismissal.itinerary.as_str() == id && dismissal.run == ended.run)
        .map(|dismissal| layover_http::Dismissal {
            run_id: dismissal.run.as_str().to_owned(),
            by: dismissal.by.clone(),
            at: dismissal.at.to_string(),
            note: dismissal.note.clone(),
        })
}
